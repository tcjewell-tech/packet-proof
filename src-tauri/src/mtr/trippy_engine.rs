use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use trippy_core::{Builder, PrivilegeMode, Protocol, ProbeStatus, Round};

use crate::mtr::engine::{EngineError, EngineHandle, ProbeEngine, RawHop, RoundResult};

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

        let (_tracer, join_handle) = tracer
            .spawn_with(move |round: &Round| {
                on_round(RoundResult { hops: round_to_raw_hops(round) });
            })
            .map_err(|e| EngineError::Other(format!("trippy spawn failed: {e}")))?;

        Ok(EngineHandle {
            stop: Box::new(move || {
                // Dropping the join handle does not stop the background thread;
                // trippy's Tracer has no public stop() at time of writing, so we
                // detach it. The thread exits when the process does (this app
                // never needs to stop a single tracer without stopping the app,
                // except when SessionRunner (Task 9) restarts a session at a new
                // rate — see that task for how it works around this).
                drop(join_handle);
            }),
        })
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
