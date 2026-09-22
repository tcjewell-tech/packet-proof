use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Emitter, State};

use crate::mtr::engine::{HopAggregator, ProbeEngine};
use crate::mtr::trippy_engine::TrippyEngine;

pub struct DemoState(pub Mutex<Option<HopAggregator>>);

#[tauri::command]
pub fn start_demo_session(app: AppHandle, state: State<DemoState>) -> Result<(), String> {
    let target = "1.1.1.1".parse().unwrap();
    let aggregator = std::sync::Arc::new(Mutex::new(HopAggregator::new()));
    let agg_for_callback = aggregator.clone();

    let engine = TrippyEngine;
    let app_for_callback = app.clone();
    let _handle = engine
        .spawn(
            target,
            Duration::from_secs(1),
            Box::new(move |round| {
                let mut agg = agg_for_callback.lock().unwrap();
                agg.record_round(&round.hops);
                let snapshot = agg.snapshot();
                let _ = app_for_callback.emit("hop-update", snapshot);
            }),
        )
        .map_err(|e| format!("{e:?}"))?;

    *state.0.lock().unwrap() = Some(HopAggregator::new());
    // The handle is intentionally leaked for this temporary demo command;
    // Task 9's SessionRunner owns proper lifecycle management (start/stop).
    std::mem::forget(_handle);
    Ok(())
}
