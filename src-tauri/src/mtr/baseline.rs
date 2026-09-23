use std::net::IpAddr;

pub const CLOUDFLARE_DNS: &str = "1.1.1.1";
pub const GOOGLE_DNS: &str = "8.8.8.8";

pub fn fixed_targets(gateway: Option<IpAddr>) -> Vec<IpAddr> {
    let mut targets = vec![];
    if let Some(gw) = gateway {
        targets.push(gw);
    }
    targets.push(CLOUDFLARE_DNS.parse().unwrap());
    targets.push(GOOGLE_DNS.parse().unwrap());
    targets
}

#[cfg(target_os = "windows")]
pub fn default_gateway() -> Option<IpAddr> {
    // GetBestRoute2/GetIpForwardTable2 is the correct low-level API, but for
    // the MVP the simplest reliable source is `route print` output, which
    // Windows always has available without extra dependencies.
    let output = std::process::Command::new("route").arg("print").arg("0.0.0.0").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find(|l| l.trim_start().starts_with("0.0.0.0"))
        .and_then(|l| l.split_whitespace().nth(2))
        .and_then(|s| s.parse().ok())
}

#[cfg(target_os = "macos")]
pub fn default_gateway() -> Option<IpAddr> {
    let output = std::process::Command::new("route").arg("-n").arg("get").arg("default").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find(|l| l.trim_start().starts_with("gateway:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
}

pub struct BaselineLogger {
    stop_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl BaselineLogger {
    pub fn start(
        engine: std::sync::Arc<dyn crate::mtr::engine::ProbeEngine>,
        targets: Vec<IpAddr>,
        record_sink: std::sync::mpsc::Sender<crate::db::ProbeRecord>,
    ) -> Self {
        let stop_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_flag_for_thread = stop_flag.clone();

        std::thread::spawn(move || {
            use std::sync::atomic::Ordering;
            // The raw-socket ICMP work below and Tauri's WebView2/window setup
            // both do heavy native, concurrent OS-level initialization; starting
            // it the instant this thread spawns races that startup and reliably
            // segfaults on Windows (observed via repeated crashes at a fixed
            // offset in release builds, gone the instant this thread is made to
            // start even slightly later). Giving the window a moment to finish
            // initializing before the first probe avoids the race entirely.
            std::thread::sleep(std::time::Duration::from_secs(2));
            while !stop_flag_for_thread.load(Ordering::Relaxed) {
                for target in &targets {
                    if stop_flag_for_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    run_single_probe_round(&engine, *target, &record_sink);
                }
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        });

        Self { stop_flag }
    }

    pub fn stop(self) {
        self.stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The round duration used for a single baseline sample. Matches the interval
/// the old spawn-based implementation used, so the data a baseline tick
/// records is unchanged.
const BASELINE_ROUND_DURATION: std::time::Duration = std::time::Duration::from_millis(200);

fn run_single_probe_round(
    engine: &std::sync::Arc<dyn crate::mtr::engine::ProbeEngine>,
    target: IpAddr,
    record_sink: &std::sync::mpsc::Sender<crate::db::ProbeRecord>,
) {
    use crate::mtr::engine::HopAggregator;

    // One bounded round, not spawn-then-stop: `spawn` starts a tracer with
    // `max_rounds(None)` whose thread cannot actually be stopped, so the old
    // spawn/poll/stop shape leaked one live tracer (thread + raw socket +
    // ongoing ICMP) per target per 60s tick, forever. `probe_once` runs a
    // single round and returns once it is done, leaving nothing behind.
    let Ok(round) = engine.probe_once(target, BASELINE_ROUND_DURATION) else {
        // A failed baseline sample is not fatal: skip this tick's record for
        // this target and let the next tick try again.
        return;
    };

    let mut aggregator = HopAggregator::new();
    aggregator.record_round(&round.hops);

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    for hop in aggregator.snapshot() {
        let _ = record_sink.send(crate::db::ProbeRecord {
            session_type: "baseline".to_string(),
            target: target.to_string(),
            session_id: None,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_gateway_and_both_public_dns_targets_when_gateway_known() {
        let gw: IpAddr = "192.168.1.1".parse().unwrap();
        let targets = fixed_targets(Some(gw));
        assert_eq!(
            targets,
            vec![gw, CLOUDFLARE_DNS.parse().unwrap(), GOOGLE_DNS.parse().unwrap()]
        );
    }

    #[test]
    fn omits_gateway_when_unknown() {
        let targets = fixed_targets(None);
        assert_eq!(
            targets,
            vec![CLOUDFLARE_DNS.parse::<IpAddr>().unwrap(), GOOGLE_DNS.parse().unwrap()]
        );
    }

    use crate::mtr::engine::{EngineError, EngineHandle, ProbeEngine, RawHop, RoundResult};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// Counts both entry points so a test can assert the baseline path takes
    /// the bounded, self-terminating one and never starts a long-lived tracer.
    struct CountingEngine {
        spawns: AtomicUsize,
        single_rounds: AtomicUsize,
    }

    impl ProbeEngine for CountingEngine {
        fn spawn(
            &self,
            _target: IpAddr,
            _round_interval: Duration,
            _on_round: Box<dyn Fn(RoundResult) + Send + Sync>,
        ) -> Result<EngineHandle, EngineError> {
            self.spawns.fetch_add(1, Ordering::SeqCst);
            Ok(EngineHandle { stop: Box::new(|| {}) })
        }

        fn probe_once(&self, _target: IpAddr, _round_duration: Duration) -> Result<RoundResult, EngineError> {
            self.single_rounds.fetch_add(1, Ordering::SeqCst);
            Ok(RoundResult {
                hops: vec![
                    RawHop { hop_index: 1, addr: Some("192.168.1.1".parse().unwrap()), rtt_ms: Some(1.0) },
                    RawHop { hop_index: 2, addr: Some("1.1.1.1".parse().unwrap()), rtt_ms: Some(12.0) },
                ],
            })
        }
    }

    #[test]
    fn a_baseline_sample_runs_one_bounded_round_and_starts_no_tracer() {
        let engine = Arc::new(CountingEngine { spawns: AtomicUsize::new(0), single_rounds: AtomicUsize::new(0) });
        let (tx, rx) = std::sync::mpsc::channel();

        run_single_probe_round(
            &(engine.clone() as Arc<dyn ProbeEngine>),
            "1.1.1.1".parse().unwrap(),
            &tx,
        );

        assert_eq!(engine.single_rounds.load(Ordering::SeqCst), 1);
        assert_eq!(
            engine.spawns.load(Ordering::SeqCst),
            0,
            "baseline must not start a continuous tracer it cannot stop"
        );

        drop(tx);
        let records: Vec<_> = rx.into_iter().collect();
        assert_eq!(records.len(), 2, "one record per hop");
        assert!(records.iter().all(|r| r.session_type == "baseline" && r.session_id.is_none()));
        assert_eq!(records[1].hop_ip.as_deref(), Some("1.1.1.1"));
    }

    #[test]
    fn a_failing_probe_records_nothing_and_does_not_panic() {
        struct FailingEngine;
        impl ProbeEngine for FailingEngine {
            fn spawn(
                &self,
                _t: IpAddr,
                _i: Duration,
                _c: Box<dyn Fn(RoundResult) + Send + Sync>,
            ) -> Result<EngineHandle, EngineError> {
                unreachable!("baseline must not call spawn")
            }
            fn probe_once(&self, _t: IpAddr, _d: Duration) -> Result<RoundResult, EngineError> {
                Err(EngineError::Other("no route to host".to_string()))
            }
        }

        let engine: Arc<dyn ProbeEngine> = Arc::new(FailingEngine);
        let (tx, rx) = std::sync::mpsc::channel();
        run_single_probe_round(&engine, "1.1.1.1".parse().unwrap(), &tx);
        drop(tx);
        assert_eq!(rx.into_iter().count(), 0);
    }
}
