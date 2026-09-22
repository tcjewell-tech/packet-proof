use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, State};

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
    let app_for_end = app.clone();
    let runner = SessionRunner::start(
        Arc::new(TrippyEngine),
        target,
        Box::new(move |snapshot| {
            let _ = app_for_callback.emit("hop-update", snapshot);
        }),
        db_sink.0.clone(),
        duration_secs.map(std::time::Duration::from_secs),
        // A timed session ends on its own. Clear the managed state here —
        // exactly what `stop_session` does — or the finished runner sits in
        // state forever and every later `start_session` is rejected as
        // "a session is already running". Then tell the frontend so it can
        // put its controls back the way a manual stop would.
        Box::new(move || {
            let _ = app_for_end.state::<SessionState>().0.lock().unwrap().take();
            let _ = app_for_end.emit("session-ended", ());
        }),
    );
    let _ = app.emit("session-started", serde_json::json!({ "session_id": runner.session_id }));
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

/// The bundled report fonts.
///
/// `tauri.conf.json` ships `fonts/*` as a bundle resource; this resolves it
/// against the installed app's resource directory. The old `"./fonts"` was
/// relative to the process CWD, which is only the crate root under
/// `cargo tauri dev`/`cargo test` — in a packaged build it resolved to
/// wherever the user happened to launch from, and the files were not shipped
/// at all.
fn fonts_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .resolve("fonts", tauri::path::BaseDirectory::Resource)
        .map_err(|e| format!("could not locate bundled report fonts: {e}"))
}

/// Reduce a frontend-supplied string to something safe to interpolate into a
/// filename: ASCII alphanumerics plus `-` and `_`, everything else folded to
/// `-`. Callers pass values that are structurally constrained already (a UUID,
/// an IP address), so this is defence in depth against path traversal and
/// reserved characters rather than a load-bearing check.
fn sanitize_filename_component(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    if cleaned.is_empty() {
        "unnamed".to_string()
    } else {
        cleaned
    }
}

#[tauri::command]
pub fn export_session_report(app: AppHandle, db: State<DbHandle>, session_id: String) -> Result<String, String> {
    let records = {
        let conn = db.0.lock().unwrap();
        conn.query_session(&session_id).map_err(|e| e.to_string())?
    };
    if records.is_empty() {
        return Err("no records found for that session".to_string());
    }

    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("reports");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let safe_session_id = sanitize_filename_component(&session_id);
    let out_path = dir.join(format!("session-{safe_session_id}.pdf"));

    crate::report::generate_session_report(&records, &fonts_dir(&app)?, &out_path)?;
    Ok(out_path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn export_trend_report(
    app: AppHandle,
    db: State<DbHandle>,
    target: String,
    since: i64,
    until: i64,
    range_label: String,
) -> Result<String, String> {
    let records = {
        let conn = db.0.lock().unwrap();
        conn.query_history(&target, Some("baseline"), since, until).map_err(|e| e.to_string())?
    };
    if records.is_empty() {
        return Err("no baseline records found for that range".to_string());
    }

    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("reports");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let safe_target = sanitize_filename_component(&target);
    let out_path = dir.join(format!("trend-{safe_target}.pdf"));

    crate::report::generate_trend_report(&records, &target, &range_label, &fonts_dir(&app)?, &out_path)?;
    Ok(out_path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::sanitize_filename_component;

    #[test]
    fn keeps_ordinary_identifiers_intact() {
        assert_eq!(
            sanitize_filename_component("5f2a1b3c-4d5e-6f70-8192-a3b4c5d6e7f8"),
            "5f2a1b3c-4d5e-6f70-8192-a3b4c5d6e7f8"
        );
        assert_eq!(sanitize_filename_component("snake_case_1"), "snake_case_1");
    }

    #[test]
    fn folds_path_separators_and_traversal() {
        assert_eq!(sanitize_filename_component("../../etc/passwd"), "------etc-passwd");
        assert_eq!(sanitize_filename_component(r"..\..\windows"), "------windows");
    }

    #[test]
    fn folds_ip_punctuation_the_old_partial_filter_missed() {
        assert_eq!(sanitize_filename_component("1.1.1.1"), "1-1-1-1");
        assert_eq!(sanitize_filename_component("2001:db8::1"), "2001-db8--1");
    }

    #[test]
    fn never_produces_an_empty_component() {
        assert_eq!(sanitize_filename_component(""), "unnamed");
    }
}
