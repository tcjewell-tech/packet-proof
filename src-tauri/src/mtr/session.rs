use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::mtr::engine::{EngineError, EngineHandle, HopAggregator, HopSnapshot, ProbeEngine, RawHop, RoundResult};
use crate::mtr::rate::AdaptiveRate;

pub struct SessionRunner {
    stop: Option<Box<dyn FnOnce() + Send>>,
}

struct Shared {
    engine: Arc<dyn ProbeEngine>,
    target: IpAddr,
    aggregator: Mutex<HopAggregator>,
    rate: Mutex<AdaptiveRate>,
    on_snapshot: Box<dyn Fn(Vec<HopSnapshot>) + Send + Sync>,
    current_handle: Mutex<Option<EngineHandle>>,
}

impl SessionRunner {
    pub fn start(
        engine: Arc<dyn ProbeEngine>,
        target: IpAddr,
        on_snapshot: Box<dyn Fn(Vec<HopSnapshot>) + Send + Sync>,
    ) -> Self {
        let shared = Arc::new(Shared {
            engine,
            target,
            aggregator: Mutex::new(HopAggregator::new()),
            rate: Mutex::new(AdaptiveRate::new()),
            on_snapshot,
            current_handle: Mutex::new(None),
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
        let _runner = SessionRunner::start(engine.clone(), "1.1.1.1".parse().unwrap(), Box::new(|_| {}));
        assert_eq!(*engine.spawns.lock().unwrap(), vec![Duration::from_millis(1000)]);
    }

    #[test]
    fn forwards_hop_snapshots_to_the_callback() {
        let engine = FakeEngine::new();
        let received = Arc::new(Mutex::new(None));
        let received_for_cb = received.clone();
        let _runner = SessionRunner::start(
            engine.clone(),
            "1.1.1.1".parse().unwrap(),
            Box::new(move |snap| *received_for_cb.lock().unwrap() = Some(snap)),
        );

        engine.drive_round(vec![hop(1, Some("1.1.1.1"), Some(5.0))]);

        let snap = received.lock().unwrap().clone().unwrap();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].hop_index, 1);
    }

    #[test]
    fn restarts_engine_with_new_interval_once_rate_backs_off() {
        let engine = FakeEngine::new();
        let _runner = SessionRunner::start(engine.clone(), "1.1.1.1".parse().unwrap(), Box::new(|_| {}));

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
