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

fn run_single_probe_round(
    engine: &std::sync::Arc<dyn crate::mtr::engine::ProbeEngine>,
    target: IpAddr,
    record_sink: &std::sync::mpsc::Sender<crate::db::ProbeRecord>,
) {
    use crate::mtr::engine::HopAggregator;
    use std::sync::{Arc, Mutex};

    let aggregator = Arc::new(Mutex::new(HopAggregator::new()));
    let agg_for_callback = aggregator.clone();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done_for_callback = done.clone();

    let handle = engine
        .spawn(
            target,
            std::time::Duration::from_millis(200),
            Box::new(move |round| {
                agg_for_callback.lock().unwrap().record_round(&round.hops);
                done_for_callback.store(true, std::sync::atomic::Ordering::Relaxed);
            }),
        )
        .expect("baseline probe spawn failed");

    // One round is enough for a baseline sample; poll briefly for it, then
    // tear the tracer down rather than leaving 3 tracers running between
    // every 60s tick.
    for _ in 0..25 {
        if done.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    (handle.stop)();

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    for hop in aggregator.lock().unwrap().snapshot() {
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
}
