use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::mtr::zone::{classify_hops, Zone};

#[derive(Debug, Clone)]
pub struct RawHop {
    pub hop_index: u8,
    pub addr: Option<IpAddr>,
    pub rtt_ms: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct RoundResult {
    pub hops: Vec<RawHop>,
}

#[derive(Debug)]
pub enum EngineError {
    Other(String),
}

/// A handle to a running probe engine.
///
/// # What `stop()` guarantees
///
/// Calling `stop()` guarantees that the `on_round` callback handed to
/// [`ProbeEngine::spawn`] **will not be invoked again** after `stop()`
/// returns — so the caller's shared state (aggregators, rate controllers, DB
/// sinks) is safe from a probe that is still winding down.
///
/// # What it does *not* guarantee
///
/// It does not guarantee that the underlying probe thread has terminated or
/// that packets have stopped going out on the wire. `trippy_core::Tracer`
/// exposes no cancellation API and is internally `Arc`-backed, so a tracer
/// started with `max_rounds(None)` keeps its worker thread and raw socket
/// until the process exits. The callback gating (see [`gate_on_stop`]) is
/// what makes the guarantee above hold anyway.
///
/// Callers that need a genuinely terminating probe should ask for a bounded
/// number of rounds instead — see [`ProbeEngine::probe_once`], which runs
/// exactly one round and whose thread ends naturally.
pub struct EngineHandle {
    pub stop: Box<dyn FnOnce() + Send>,
}

/// Wraps a round callback so it becomes a permanent no-op once the returned
/// flag is set, and hands back the flag for an [`EngineHandle::stop`] closure
/// to flip.
///
/// Every [`ProbeEngine`] implementation is expected to route its callback
/// through this and to set the flag as the first thing `stop()` does. That is
/// what makes `stop()`'s "the callback will not fire again" contract hold for
/// engines (like trippy-core) whose worker thread cannot actually be killed:
/// the detached thread may keep running, but it can no longer touch any state
/// the caller owns.
pub fn gate_on_stop(
    on_round: Box<dyn Fn(RoundResult) + Send + Sync>,
) -> (Box<dyn Fn(RoundResult) + Send + Sync>, Arc<AtomicBool>) {
    let stopped = Arc::new(AtomicBool::new(false));
    let stopped_for_cb = stopped.clone();
    let gated = Box::new(move |round: RoundResult| {
        if stopped_for_cb.load(Ordering::SeqCst) {
            return;
        }
        on_round(round);
    });
    (gated, stopped)
}

pub trait ProbeEngine: Send + Sync {
    fn spawn(
        &self,
        target: IpAddr,
        round_interval: Duration,
        on_round: Box<dyn Fn(RoundResult) + Send + Sync>,
    ) -> Result<EngineHandle, EngineError>;

    /// Run exactly **one** probe round against `target`, blocking until it
    /// completes (or fails), and return its result.
    ///
    /// Unlike [`ProbeEngine::spawn`] this leaves nothing running behind it:
    /// the underlying tracer is bounded to a single round, so its thread and
    /// socket are released before this returns. Callers that only want a
    /// periodic sample (the baseline logger) must use this rather than
    /// spawn-then-stop, which cannot actually stop anything.
    ///
    /// `round_duration` bounds how long the single round may take.
    fn probe_once(&self, target: IpAddr, round_duration: Duration) -> Result<RoundResult, EngineError>;
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HopSnapshot {
    pub hop_index: u8,
    pub addr: Option<IpAddr>,
    pub rtt_best_ms: Option<f64>,
    pub rtt_avg_ms: Option<f64>,
    pub rtt_worst_ms: Option<f64>,
    pub loss_pct: f64,
    pub zone: Zone,
}

struct HopAccumulator {
    sent: u32,
    received: u32,
    rtt_sum_ms: f64,
    rtt_best_ms: Option<f64>,
    rtt_worst_ms: Option<f64>,
    last_addr: Option<IpAddr>,
}

impl HopAccumulator {
    fn new() -> Self {
        Self { sent: 0, received: 0, rtt_sum_ms: 0.0, rtt_best_ms: None, rtt_worst_ms: None, last_addr: None }
    }

    fn record(&mut self, hop: &RawHop) {
        self.sent += 1;
        if let Some(addr) = hop.addr {
            self.last_addr = Some(addr);
        }
        if let Some(rtt) = hop.rtt_ms {
            self.received += 1;
            self.rtt_sum_ms += rtt;
            self.rtt_best_ms = Some(self.rtt_best_ms.map_or(rtt, |b: f64| b.min(rtt)));
            self.rtt_worst_ms = Some(self.rtt_worst_ms.map_or(rtt, |w: f64| w.max(rtt)));
        }
    }

    fn snapshot(&self, hop_index: u8, zone: Zone) -> HopSnapshot {
        let loss_pct = if self.sent == 0 {
            0.0
        } else {
            100.0 * (1.0 - self.received as f64 / self.sent as f64)
        };
        HopSnapshot {
            hop_index,
            addr: self.last_addr,
            rtt_best_ms: self.rtt_best_ms,
            rtt_avg_ms: if self.received == 0 { None } else { Some(self.rtt_sum_ms / self.received as f64) },
            rtt_worst_ms: self.rtt_worst_ms,
            loss_pct,
            zone,
        }
    }
}

pub struct HopAggregator {
    hops: BTreeMap<u8, HopAccumulator>,
}

impl HopAggregator {
    pub fn new() -> Self {
        Self { hops: BTreeMap::new() }
    }

    pub fn record_round(&mut self, hops: &[RawHop]) {
        for hop in hops {
            self.hops.entry(hop.hop_index).or_insert_with(HopAccumulator::new).record(hop);
        }
    }

    pub fn snapshot(&self) -> Vec<HopSnapshot> {
        let addrs: Vec<Option<IpAddr>> = self.hops.values().map(|acc| acc.last_addr).collect();
        let zones = classify_hops(&addrs);

        self.hops.iter().zip(zones).map(|((idx, acc), zone)| acc.snapshot(*idx, zone)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hop(idx: u8, addr: Option<&str>, rtt: Option<f64>) -> RawHop {
        RawHop { hop_index: idx, addr: addr.map(|a| a.parse().unwrap()), rtt_ms: rtt }
    }

    #[test]
    fn empty_aggregator_has_no_hops() {
        let agg = HopAggregator::new();
        assert_eq!(agg.snapshot(), vec![]);
    }

    #[test]
    fn single_round_single_hop() {
        let mut agg = HopAggregator::new();
        agg.record_round(&[hop(1, Some("192.168.1.1"), Some(5.0))]);
        let snap = agg.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].hop_index, 1);
        assert_eq!(snap[0].addr, Some("192.168.1.1".parse().unwrap()));
        assert_eq!(snap[0].rtt_best_ms, Some(5.0));
        assert_eq!(snap[0].rtt_avg_ms, Some(5.0));
        assert_eq!(snap[0].rtt_worst_ms, Some(5.0));
        assert_eq!(snap[0].loss_pct, 0.0);
    }

    #[test]
    fn best_avg_worst_across_rounds() {
        let mut agg = HopAggregator::new();
        agg.record_round(&[hop(1, Some("1.1.1.1"), Some(10.0))]);
        agg.record_round(&[hop(1, Some("1.1.1.1"), Some(30.0))]);
        agg.record_round(&[hop(1, Some("1.1.1.1"), Some(20.0))]);
        let snap = agg.snapshot();
        assert_eq!(snap[0].rtt_best_ms, Some(10.0));
        assert_eq!(snap[0].rtt_worst_ms, Some(30.0));
        assert_eq!(snap[0].rtt_avg_ms, Some(20.0));
    }

    #[test]
    fn loss_pct_counts_non_responding_rounds() {
        let mut agg = HopAggregator::new();
        agg.record_round(&[hop(1, Some("1.1.1.1"), Some(10.0))]);
        agg.record_round(&[hop(1, None, None)]);
        agg.record_round(&[hop(1, Some("1.1.1.1"), Some(10.0))]);
        agg.record_round(&[hop(1, None, None)]);
        let snap = agg.snapshot();
        assert_eq!(snap[0].loss_pct, 50.0);
    }

    #[test]
    fn last_known_addr_is_kept_after_a_loss() {
        let mut agg = HopAggregator::new();
        agg.record_round(&[hop(1, Some("1.1.1.1"), Some(10.0))]);
        agg.record_round(&[hop(1, None, None)]);
        let snap = agg.snapshot();
        assert_eq!(snap[0].addr, Some("1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn snapshot_is_sorted_by_hop_index() {
        let mut agg = HopAggregator::new();
        agg.record_round(&[hop(2, Some("2.2.2.2"), Some(2.0)), hop(1, Some("1.1.1.1"), Some(1.0))]);
        let snap = agg.snapshot();
        assert_eq!(snap.iter().map(|h| h.hop_index).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn gated_callback_fires_until_the_flag_is_set_then_never_again() {
        let calls = Arc::new(std::sync::Mutex::new(0usize));
        let calls_for_cb = calls.clone();
        let (gated, stopped) = gate_on_stop(Box::new(move |_| *calls_for_cb.lock().unwrap() += 1));

        gated(RoundResult { hops: vec![] });
        assert_eq!(*calls.lock().unwrap(), 1);

        stopped.store(true, Ordering::SeqCst);
        gated(RoundResult { hops: vec![] });
        gated(RoundResult { hops: vec![] });
        assert_eq!(*calls.lock().unwrap(), 1, "callback must not fire after stop");
    }

    #[test]
    fn snapshot_includes_zone_classification() {
        let mut agg = HopAggregator::new();
        agg.record_round(&[hop(1, Some("192.168.1.1"), Some(1.0)), hop(2, Some("203.0.113.1"), Some(5.0))]);
        let snap = agg.snapshot();
        assert_eq!(snap[0].zone, crate::mtr::zone::Zone::Home);
        assert_eq!(snap[1].zone, crate::mtr::zone::Zone::IspGateway);
    }
}
