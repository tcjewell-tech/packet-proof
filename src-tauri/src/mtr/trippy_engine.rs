use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Duration;

use trippy_core::{Builder, PrivilegeMode, ProbeStatus, Protocol, Round};

use crate::mtr::engine::{gate_on_stop, EngineError, EngineHandle, ProbeEngine, RawHop, RoundResult};

pub struct TrippyEngine;

impl ProbeEngine for TrippyEngine {
    fn spawn(
        &self,
        target: IpAddr,
        round_interval: Duration,
        on_round: Box<dyn Fn(RoundResult) + Send + Sync>,
    ) -> Result<EngineHandle, EngineError> {
        let tracer = Builder::new(target)
            .protocol(Protocol::Icmp)
            .privilege_mode(PrivilegeMode::Privileged)
            .max_rounds(None)
            .min_round_duration(round_interval)
            .max_round_duration(round_interval)
            .build()
            .map_err(|e| EngineError::Other(format!("trippy build failed: {e}")))?;

        // The callback we hand trippy is gated behind a "stopped" flag so that
        // `stop()` below is a real guarantee and not a wish: trippy's `Tracer`
        // has no cancellation API and is `Arc`-backed internally, so the
        // worker thread survives both the handle drop and the `Tracer` drop.
        // Gating means a still-running detached tracer can no longer mutate
        // any caller-owned state (aggregator, rate controller, DB sink).
        let (gated, stopped) = gate_on_stop(on_round);

        let (_tracer, join_handle) = tracer
            .spawn_with(move |round: &Round| {
                gated(RoundResult { hops: round_to_raw_hops(round) });
            })
            .map_err(|e| EngineError::Other(format!("trippy spawn failed: {e}")))?;

        Ok(EngineHandle {
            stop: Box::new(move || {
                // Order matters: silence the callback *first*, then detach.
                stopped.store(true, Ordering::SeqCst);
                // Dropping the join handle detaches the thread; it keeps its
                // raw socket and keeps emitting ICMP until the process exits.
                // That residual traffic is the known, documented limitation —
                // callers who need a genuinely terminating probe use
                // `probe_once` instead.
                drop(join_handle);
            }),
        })
    }

    fn probe_once(&self, target: IpAddr, round_duration: Duration) -> Result<RoundResult, EngineError> {
        // `max_rounds(Some(1))` is what makes this genuinely terminating:
        // `run_with` returns once the single round completes, and the tracer
        // (and its socket) are dropped when this function returns.
        //
        // min == max round duration mirrors `spawn` above, so a single round
        // here samples the network exactly the way one round of a continuous
        // trace at the same interval would.
        let tracer = Builder::new(target)
            .protocol(Protocol::Icmp)
            .privilege_mode(PrivilegeMode::Privileged)
            .max_rounds(Some(1))
            .min_round_duration(round_duration)
            .max_round_duration(round_duration)
            .build()
            .map_err(|e| EngineError::Other(format!("trippy build failed: {e}")))?;

        let captured: Mutex<Option<RoundResult>> = Mutex::new(None);
        tracer
            .run_with(|round: &Round| {
                *captured.lock().unwrap() = Some(RoundResult { hops: round_to_raw_hops(round) });
            })
            .map_err(|e| EngineError::Other(format!("trippy run failed: {e}")))?;

        captured
            .into_inner()
            .unwrap()
            .ok_or_else(|| EngineError::Other("trippy completed without producing a round".to_string()))
    }
}

fn round_to_raw_hops(round: &Round) -> Vec<RawHop> {
    let max_ttl = round.largest_ttl.0;
    let mut completed: HashMap<u8, RawHop> = HashMap::new();

    for probe in round.probes {
        if let ProbeStatus::Complete(c) = probe {
            let ttl = c.ttl.0;
            let rtt_ms = c
                .received
                .duration_since(c.sent)
                .unwrap_or_default()
                .as_secs_f64()
                * 1000.0;
            completed.insert(ttl, RawHop { hop_index: ttl, addr: Some(c.host), rtt_ms: Some(rtt_ms) });
        }
    }

    (1..=max_ttl)
        .map(|ttl| completed.remove(&ttl).unwrap_or(RawHop { hop_index: ttl, addr: None, rtt_ms: None }))
        .collect()
}
