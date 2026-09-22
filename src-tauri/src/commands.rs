use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, State};

use crate::mtr::baseline::{default_gateway, fixed_targets, BaselineLogger};
use crate::mtr::session::SessionRunner;
use crate::mtr::trippy_engine::TrippyEngine;
use crate::procnet::{ConnectionQuery, ProcessInfo};

pub struct SessionState(pub Mutex<Option<SessionRunner>>);

pub struct BaselineState(pub Mutex<Option<BaselineLogger>>);

pub struct DbSink(pub std::sync::mpsc::Sender<crate::db::ProbeRecord>);

// `rusqlite::Connection` is not `Sync` (its statement cache uses `RefCell`
// internally), so a plain `Arc<Db>` cannot satisfy Tauri's `State<T: Send +
// Sync>` bound. Wrap it in a `Mutex`, matching the pattern already used by
// `SessionState`/`BaselineState` for shared mutable state.
pub struct DbHandle(pub Arc<Mutex<crate::db::Db>>);

#[tauri::command]
pub fn start_session(
    app: AppHandle,
    state: State<SessionState>,
    db_sink: State<DbSink>,
    target: String,
    duration_secs: Option<u64>,
) -> Result<(), String> {
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
        db_sink.0.clone(),
        duration_secs.map(std::time::Duration::from_secs),
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

#[tauri::command]
pub fn start_baseline(state: State<BaselineState>, db_sink: State<DbSink>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    if guard.is_some() {
        return Err("baseline is already running".to_string());
    }
    let targets = fixed_targets(default_gateway());
    let logger = BaselineLogger::start(Arc::new(TrippyEngine), targets, db_sink.0.clone());
    *guard = Some(logger);
    Ok(())
}

#[tauri::command]
pub fn stop_baseline(state: State<BaselineState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    if let Some(logger) = guard.take() {
        logger.stop();
        Ok(())
    } else {
        Err("baseline is not running".to_string())
    }
}

#[tauri::command]
pub fn list_candidate_processes() -> Vec<ProcessInfo> {
    crate::procnet::list_candidate_processes()
}

#[tauri::command]
pub fn list_connections_for_pid(pid: u32) -> ConnectionQuery {
    crate::procnet::connections_for_pid(pid)
}

#[tauri::command]
pub fn get_baseline_history(
    db: State<DbHandle>,
    target: String,
    since: i64,
    until: i64,
) -> Result<Vec<crate::db::ProbeRecord>, String> {
    let conn = db.0.lock().unwrap();
    conn.query_history(&target, Some("baseline"), since, until)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_session_history(
    db: State<DbHandle>,
    session_id: String,
) -> Result<Vec<crate::db::ProbeRecord>, String> {
    let conn = db.0.lock().unwrap();
    conn.query_session(&session_id).map_err(|e| e.to_string())
}
