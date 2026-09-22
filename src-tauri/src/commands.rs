use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, State};

use crate::mtr::session::SessionRunner;
use crate::mtr::trippy_engine::TrippyEngine;

pub struct SessionState(pub Mutex<Option<SessionRunner>>);

#[tauri::command]
pub fn start_session(app: AppHandle, state: State<SessionState>, target: String) -> Result<(), String> {
    let target: IpAddr = target.parse().map_err(|_| "invalid target IP".to_string())?;
    let mut guard = state.0.lock().unwrap();
    if guard.is_some() {
        return Err("a session is already running".to_string());
    }

    let app_for_callback = app.clone();
    let runner = SessionRunner::start(
        Arc::new(TrippyEngine),
        target,
        Box::new(move |snapshot| {
            let _ = app_for_callback.emit("hop-update", snapshot);
        }),
    );
    *guard = Some(runner);
    Ok(())
}

#[tauri::command]
pub fn stop_session(state: State<SessionState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    if let Some(runner) = guard.take() {
        runner.stop();
        Ok(())
    } else {
        Err("no session is running".to_string())
    }
}
