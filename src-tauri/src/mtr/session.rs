use std::net::IpAddr;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::db::ProbeRecord;
use crate::mtr::engine::{EngineError, EngineHandle, HopAggregator, HopSnapshot, ProbeEngine, RawHop, RoundResult};
use crate::mtr::rate::AdaptiveRate;

pub struct SessionRunner {
    stop: Option<Box<dyn FnOnce() + Send>>,
    pub session_id: String,
}

struct Shared {
    engine: Arc<dyn ProbeEngine>,
    target: IpAddr,
    target_label: String,
    session_id: String,
    aggregator: Mutex<HopAggregator>,
    rate: Mutex<AdaptiveRate>,
    on_snapshot: Box<dyn Fn(Vec<HopSnapshot>) + Send + Sync>,
    current_handle: Mutex<Option<EngineHandle>>,
    record_sink: Sender<ProbeRecord>,
    /// Set by whichever of the manual stop and the duration timer gets there
    /// first, and never cleared. Read by `spawn_with_current_interval`, so a
    /// round already past the engine's own gate cannot restart the tracer
    /// after the session has ended.
    stopped: std::sync::atomic::AtomicBool,
}

impl SessionRunner {
    /// Start a session.
    ///
    /// `on_end` is invoked **only** when a `duration` timer fires and stops
    /// the session by itself — never on an explicit [`SessionRunner::stop`].
    /// It exists so the owner of the runner (the Tauri managed
    /// `SessionState`) can drop the now-finished runner and tell the frontend;
    /// without it a timed session leaves a dead runner parked in state and
    /// `start_session` keeps rejecting new sessions as "already running".
    pub fn start(
        engine: Arc<dyn ProbeEngine>,
        target: IpAddr,
        on_snapshot: Box<dyn Fn(Vec<HopSnapshot>) + Send + Sync>,
        record_sink: Sender<ProbeRecord>,
        duration: Option<Duration>,
        on_end: Box<dyn FnOnce() + Send>,
    ) -> Self {
        let shared = Arc::new(Shared {
            engine,
            target,
            target_label: target.to_string(),
            session_id: uuid::Uuid::new_v4().to_string(),
            aggregator: Mutex::new(HopAggregator::new()),
            rate: Mutex::new(AdaptiveRate::new()),
            on_snapshot,
            current_handle: Mutex::new(None),
            record_sink,
            stopped: std::sync::atomic::AtomicBool::new(false),
        });

        spawn_with_current_interval(&shared);

        if let Some(duration) = duration {
            let shared_for_timer = shared.clone();
            std::thread::spawn(move || {
                std::thread::sleep(duration);
                // Claim the stop. If a manual stop got there first this fails
                // and the timer has nothing to end and nobody to tell.
                use std::sync::atomic::Ordering::SeqCst;
                if shared_for_timer.stopped.compare_exchange(false, true, SeqCst, SeqCst).is_err() {
                    return;
                }
                if let Some(handle) = shared_for_timer.current_handle.lock().unwrap().take() {
                    (handle.stop)();
                }
                on_end();
            });
        }

        let session_id = shared.session_id.clone();

        SessionRunner {
            stop: Some(Box::new(move || {
                // Flag first, handle second: `spawn_with_current_interval`
                // reads the flag while holding `current_handle`, so whichever
                // of the two gets the lock first, the session ends up with no
                // live, unstopped tracer.
                shared.stopped.store(true, std::sync::atomic::Ordering::SeqCst);
                if let Some(handle) = shared.current_handle.lock().unwrap().take() {
                    // Gated at the engine level: after this returns the round
                    // callback is a permanent no-op, so nothing else touches
                    // the aggregator, the rate controller or the DB sink under
                    // this session's id.
                    (handle.stop)();
                }
            })),
            session_id,
        }
    }

    pub fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

fn spawn_with_current_interval(shared: &Arc<Shared>) {
    let interval = shared.rate.lock().unwrap().current_interval();
    let shared_for_callback = shared.clone();

    // The check and the store happen under the same lock the stop path takes,
    // so a round that was already in flight when the session stopped cannot
    // sneak a fresh tracer in behind it. Without this, the adaptive-rate
    // restart could start a tracer nobody holds a handle to, and that tracer
    // would go on writing rows under a "stopped" session's id.
    let mut slot = shared.current_handle.lock().unwrap();
    if shared.stopped.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }

    let handle = shared
        .engine
        .spawn(
            shared.target,
            interval,
            Box::new(move |round| on_round(&shared_for_callback, round)),
        )
        .expect("engine spawn failed");

    *slot = Some(handle);
}

fn on_round(shared: &Arc<Shared>, round: RoundResult) {
    shared.aggregator.lock().unwrap().record_round(&round.hops);

    let last_hop_lost = round.hops.last().map(|h| h.addr.is_none()).unwrap_or(false);
    let interval_before = shared.rate.lock().unwrap().current_interval();
    shared.rate.lock().unwrap().record_result(last_hop_lost);
    let interval_after = shared.rate.lock().unwrap().current_interval();

    let snapshot = shared.aggregator.lock().unwrap().snapshot();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    for hop in &snapshot {
        let _ = shared.record_sink.send(ProbeRecord {
            session_type: "timed".to_string(),
            target: shared.target_label.clone(),
            session_id: Some(shared.session_id.clone()),
            timestamp: now,
            hop_index: hop.hop_index,
            hop_ip: hop.addr.map(|a| a.to_string()),
            rtt_best_ms: hop.rtt_best_ms,
            rtt_avg_ms: hop.rtt_avg_ms,
            rtt_worst_ms: hop.rtt_worst_ms,
            loss_pct: hop.loss_pct,
            zone: hop.zone.as_db_str().to_string(),
        });
    }

    (shared.on_snapshot)(snapshot);

    if interval_after != interval_before {
        if let Some(old_handle) = shared.current_handle.lock().unwrap().take() {
            (old_handle.stop)();
        }
        spawn_with_current_interval(shared);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records every (target, interval) it was asked to spawn with, and lets
    /// the test drive rounds manually via the stored callback.
    ///
    /// The callback is stored as an `Arc` (rather than the `Box` the trait
    /// hands us) purely so `drive_round` can clone it out from behind the
    /// mutex and drop the lock *before* invoking it. `SessionRunner`'s
    /// restart-on-interval-change path calls back into `engine.spawn()`
    /// synchronously from within the round callback, which re-locks
    /// `last_callback` on the same thread; holding the lock across the
    /// callback call would self-deadlock on `std::sync::Mutex` (which is not
    /// reentrant). This is a test-double-only concern and doesn't change
    /// what's being exercised.
    struct FakeEngine {
        spawns: Mutex<Vec<Duration>>,
        last_callback: Mutex<Option<Arc<dyn Fn(RoundResult) + Send + Sync>>>,
    }

    impl FakeEngine {
        fn new() -> Arc<Self> {
            Arc::new(Self { spawns: Mutex::new(vec![]), last_callback: Mutex::new(None) })
        }

        fn drive_round(&self, hops: Vec<RawHop>) {
            let cb = self.last_callback.lock().unwrap().clone();
            (cb.unwrap())(RoundResult { hops });
        }

        /// Drive a round through a callback captured earlier, standing in for
        /// a detached trippy thread that is still alive after `stop()`.
        fn drive_round_through(cb: &Arc<dyn Fn(RoundResult) + Send + Sync>, hops: Vec<RawHop>) {
            cb(RoundResult { hops });
        }

        fn current_callback(&self) -> Arc<dyn Fn(RoundResult) + Send + Sync> {
            self.last_callback.lock().unwrap().clone().unwrap()
        }
    }

    impl ProbeEngine for FakeEngine {
        fn spawn(
            &self,
            _target: IpAddr,
            interval: Duration,
            on_round: Box<dyn Fn(RoundResult) + Send + Sync>,
        ) -> Result<EngineHandle, EngineError> {
            self.spawns.lock().unwrap().push(interval);
            // Mirror `TrippyEngine::spawn` exactly: gate the callback and flip
            // the flag from `stop()`. A no-op `stop` here is what let the
            // "stop doesn't stop anything" bug hide from every test.
            let (gated, stopped) = crate::mtr::engine::gate_on_stop(on_round);
            *self.last_callback.lock().unwrap() = Some(Arc::from(gated));
            Ok(EngineHandle {
                stop: Box::new(move || stopped.store(true, std::sync::atomic::Ordering::SeqCst)),
            })
        }

        fn probe_once(&self, _target: IpAddr, _round_duration: Duration) -> Result<RoundResult, EngineError> {
            unreachable!("SessionRunner uses spawn, not probe_once")
        }
    }

    fn hop(idx: u8, addr: Option<&str>, rtt: Option<f64>) -> RawHop {
        RawHop { hop_index: idx, addr: addr.map(|a| a.parse().unwrap()), rtt_ms: rtt }
    }

    #[test]
    fn starts_with_a_one_second_interval() {
        let engine = FakeEngine::new();
        let (tx, _rx) = std::sync::mpsc::channel();
        let _runner = SessionRunner::start(
            engine.clone(),
            "1.1.1.1".parse().unwrap(),
            Box::new(|_| {}),
            tx,
            None,
            Box::new(|| {}),
        );
        assert_eq!(*engine.spawns.lock().unwrap(), vec![Duration::from_millis(1000)]);
    }

    #[test]
    fn forwards_hop_snapshots_to_the_callback() {
        let engine = FakeEngine::new();
        let received = Arc::new(Mutex::new(None));
        let received_for_cb = received.clone();
        let (tx, _rx) = std::sync::mpsc::channel();
        let _runner = SessionRunner::start(
            engine.clone(),
            "1.1.1.1".parse().unwrap(),
            Box::new(move |snap| *received_for_cb.lock().unwrap() = Some(snap)),
            tx,
            None,
            Box::new(|| {}),
        );

        engine.drive_round(vec![hop(1, Some("1.1.1.1"), Some(5.0))]);

        let snap = received.lock().unwrap().clone().unwrap();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].hop_index, 1);
    }

    #[test]
    fn restarts_engine_with_new_interval_once_rate_backs_off() {
        let engine = FakeEngine::new();
        let (tx, _rx) = std::sync::mpsc::channel();
        let _runner = SessionRunner::start(
            engine.clone(),
            "1.1.1.1".parse().unwrap(),
            Box::new(|_| {}),
            tx,
            None,
            Box::new(|| {}),
        );

        // Last hop lost 10 times in a row -> AdaptiveRate backs off past its
        // window-full threshold, which must trigger a respawn.
        for _ in 0..10 {
            engine.drive_round(vec![hop(1, None, None)]);
        }

        let spawns = engine.spawns.lock().unwrap();
        assert_eq!(spawns.len(), 2);
        assert_eq!(spawns[1], Duration::from_millis(1500));
    }

    #[test]
    fn accepts_a_duration_without_panicking() {
        let engine = FakeEngine::new();
        let (tx, _rx) = std::sync::mpsc::channel();
        let runner = SessionRunner::start(
            engine,
            "1.1.1.1".parse().unwrap(),
            Box::new(|_| {}),
            tx,
            Some(Duration::from_millis(50)),
            Box::new(|| {}),
        );
        std::thread::sleep(Duration::from_millis(100));
        runner.stop(); // must not panic even though the auto-stop timer already fired
    }

    #[test]
    fn a_stopped_session_ignores_rounds_from_a_still_running_tracer() {
        let engine = FakeEngine::new();
        let snapshots = Arc::new(Mutex::new(Vec::<Vec<HopSnapshot>>::new()));
        let snapshots_for_cb = snapshots.clone();
        let (tx, rx) = std::sync::mpsc::channel();

        let runner = SessionRunner::start(
            engine.clone(),
            "1.1.1.1".parse().unwrap(),
            Box::new(move |snap| snapshots_for_cb.lock().unwrap().push(snap)),
            tx,
            None,
            Box::new(|| {}),
        );

        // Hold on to the callback the way a detached trippy thread would: it
        // survives `stop()` and would happily keep firing.
        let leaked_callback = engine.current_callback();

        engine.drive_round(vec![hop(1, Some("1.1.1.1"), Some(5.0))]);
        assert_eq!(snapshots.lock().unwrap().len(), 1);
        let after_one_round = snapshots.lock().unwrap().last().unwrap().clone();

        runner.stop();

        // The leaked tracer keeps delivering rounds. None of them may be seen.
        for _ in 0..5 {
            FakeEngine::drive_round_through(&leaked_callback, vec![hop(1, None, None)]);
        }

        assert_eq!(
            snapshots.lock().unwrap().len(),
            1,
            "no snapshot may be emitted after stop()"
        );
        assert_eq!(
            *snapshots.lock().unwrap().last().unwrap(),
            after_one_round,
            "the aggregator must not be mutated after stop()"
        );
        assert_eq!(
            *engine.spawns.lock().unwrap(),
            vec![Duration::from_millis(1000)],
            "a stopped session must never respawn the engine"
        );

        let records: Vec<_> = rx.try_iter().collect();
        assert_eq!(records.len(), 1, "only the pre-stop round may reach the DB sink");
    }

    /// The engine's callback gate stops a *detached* tracer from being heard,
    /// but a round that was already past the gate when `stop()` landed could
    /// still reach the adaptive-rate restart. This pins the second guard: once
    /// the session is stopped, nothing spawns a tracer nobody can stop.
    #[test]
    fn a_stopped_session_refuses_to_respawn_the_engine() {
        let engine = FakeEngine::new();
        let (tx, _rx) = std::sync::mpsc::channel();

        let shared = Arc::new(Shared {
            engine: engine.clone(),
            target: "1.1.1.1".parse().unwrap(),
            target_label: "1.1.1.1".to_string(),
            session_id: "s1".to_string(),
            aggregator: Mutex::new(HopAggregator::new()),
            rate: Mutex::new(AdaptiveRate::new()),
            on_snapshot: Box::new(|_| {}),
            current_handle: Mutex::new(None),
            record_sink: tx,
            stopped: std::sync::atomic::AtomicBool::new(false),
        });

        spawn_with_current_interval(&shared);
        assert_eq!(engine.spawns.lock().unwrap().len(), 1);

        shared.stopped.store(true, std::sync::atomic::Ordering::SeqCst);
        spawn_with_current_interval(&shared);
        spawn_with_current_interval(&shared);

        assert_eq!(
            engine.spawns.lock().unwrap().len(),
            1,
            "a stopped session must not start a tracer nobody holds a handle to"
        );
    }

    #[test]
    fn the_duration_timer_reports_that_the_session_ended_by_itself() {
        let engine = FakeEngine::new();
        let (tx, _rx) = std::sync::mpsc::channel();
        let ended = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ended_for_cb = ended.clone();

        let _runner = SessionRunner::start(
            engine,
            "1.1.1.1".parse().unwrap(),
            Box::new(|_| {}),
            tx,
            Some(Duration::from_millis(30)),
            Box::new(move || ended_for_cb.store(true, std::sync::atomic::Ordering::SeqCst)),
        );

        std::thread::sleep(Duration::from_millis(250));
        assert!(
            ended.load(std::sync::atomic::Ordering::SeqCst),
            "the auto-stop timer must report the session ended so its owner can clear it"
        );
    }

    #[test]
    fn a_manual_stop_does_not_report_an_automatic_end() {
        let engine = FakeEngine::new();
        let (tx, _rx) = std::sync::mpsc::channel();
        let ended = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ended_for_cb = ended.clone();

        let runner = SessionRunner::start(
            engine,
            "1.1.1.1".parse().unwrap(),
            Box::new(|_| {}),
            tx,
            Some(Duration::from_millis(60)),
            Box::new(move || ended_for_cb.store(true, std::sync::atomic::Ordering::SeqCst)),
        );
        runner.stop();

        std::thread::sleep(Duration::from_millis(250));
        assert!(
            !ended.load(std::sync::atomic::Ordering::SeqCst),
            "stop_session already clears the state; a second clear would be a double-take"
        );
    }
}
