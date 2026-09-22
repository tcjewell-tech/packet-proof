use std::net::IpAddr;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::db::ProbeRecord;
use crate::mtr::engine::{EngineError, EngineHandle, HopAggregator, HopSnapshot, ProbeEngine, RawHop, RoundResult};
use crate::mtr::rate::AdaptiveRate;

pub struct SessionRunner {
    stop: Option<Box<dyn FnOnce() + Send>>,
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
}

impl SessionRunner {
    pub fn start(
        engine: Arc<dyn ProbeEngine>,
        target: IpAddr,
        on_snapshot: Box<dyn Fn(Vec<HopSnapshot>) + Send + Sync>,
        record_sink: Sender<ProbeRecord>,
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
        });

        spawn_with_current_interval(&shared);

        SessionRunner {
            stop: Some(Box::new(move || {
                if let Some(handle) = shared.current_handle.lock().unwrap().take() {
                    (handle.stop)();
                }
            })),
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

    let handle = shared
        .engine
        .spawn(
            shared.target,
            interval,
            Box::new(move |round| on_round(&shared_for_callback, round)),
        )
        .expect("engine spawn failed");

    *shared.current_handle.lock().unwrap() = Some(handle);
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
    }

    impl ProbeEngine for FakeEngine {
        fn spawn(
            &self,
            _target: IpAddr,
            interval: Duration,
            on_round: Box<dyn Fn(RoundResult) + Send + Sync>,
        ) -> Result<EngineHandle, EngineError> {
            self.spawns.lock().unwrap().push(interval);
            *self.last_callback.lock().unwrap() = Some(Arc::from(on_round));
            Ok(EngineHandle { stop: Box::new(|| {}) })
        }
    }

    fn hop(idx: u8, addr: Option<&str>, rtt: Option<f64>) -> RawHop {
        RawHop { hop_index: idx, addr: addr.map(|a| a.parse().unwrap()), rtt_ms: rtt }
    }

    #[test]
    fn starts_with_a_one_second_interval() {
        let engine = FakeEngine::new();
        let (tx, _rx) = std::sync::mpsc::channel();
        let _runner =
            SessionRunner::start(engine.clone(), "1.1.1.1".parse().unwrap(), Box::new(|_| {}), tx);
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
        let _runner =
            SessionRunner::start(engine.clone(), "1.1.1.1".parse().unwrap(), Box::new(|_| {}), tx);

        // Last hop lost 10 times in a row -> AdaptiveRate backs off past its
        // window-full threshold, which must trigger a respawn.
        for _ in 0..10 {
            engine.drive_round(vec![hop(1, None, None)]);
        }

        let spawns = engine.spawns.lock().unwrap();
        assert_eq!(spawns.len(), 2);
        assert_eq!(spawns[1], Duration::from_millis(1500));
    }
}
