use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::Duration;

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

pub struct EngineHandle {
    pub stop: Box<dyn FnOnce() + Send>,
}

pub trait ProbeEngine: Send + Sync {
    fn spawn(
        &self,
        target: IpAddr,
        round_interval: Duration,
        on_round: Box<dyn Fn(RoundResult) + Send + Sync>,
    ) -> Result<EngineHandle, EngineError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct HopSnapshot {
    pub hop_index: u8,
    pub addr: Option<IpAddr>,
    pub rtt_best_ms: Option<f64>,
    pub rtt_avg_ms: Option<f64>,
    pub rtt_worst_ms: Option<f64>,
    pub loss_pct: f64,
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

    fn snapshot(&self, hop_index: u8) -> HopSnapshot {
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
        self.hops.iter().map(|(idx, acc)| acc.snapshot(*idx)).collect()
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
}
