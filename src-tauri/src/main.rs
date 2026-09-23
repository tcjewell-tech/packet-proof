#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod db;
mod db_writer;
mod elevation;
mod mtr;
mod procnet;
mod report;
mod tray;

fn main() {
    if elevation::decide_relaunch(&elevation::check_elevation()) {
        elevation::relaunch_elevated();
    }

    // Force the raw-socket/ICMP machinery's one-time OS-level setup (DLL
    // loading, WinSock init) to happen here, synchronously, on the main
    // thread, before Tauri creates the window and before the baseline
    // logger's background thread starts. Without this, that first-use
    // initialization can run concurrently on two threads at once — this
    // thread via WebView2/window creation touching its own set of system
    // DLLs, the baseline thread via its first probe — and that race
    // reliably segfaults release builds (reproduced repeatedly; never in
    // debug, never when only one of the two runs, never in unit tests,
    // which only ever exercise a fake probe engine). Doing the first
    // socket-touching call here, alone, closes the race.
    let _ = mtr::engine::ProbeEngine::probe_once(
        &mtr::trippy_engine::TrippyEngine,
        std::net::IpAddr::from([127, 0, 0, 1]),
        std::time::Duration::from_millis(200),
    );

    tauri::Builder::default()
        .setup(|app| {
            use tauri::Manager;
            let db_path = app.path().app_data_dir()?.join("packet_proof.sqlite");
            std::fs::create_dir_all(db_path.parent().unwrap())?;
            // .clone() here, not a plain move: Task 18 reuses `db_path` later in
            // this same closure to open a second, read-only connection.
            let sink = db_writer::spawn_db_writer(db_path.clone());
            app.manage(commands::DbSink(sink.clone()));

            let targets = mtr::baseline::fixed_targets(mtr::baseline::default_gateway());
            let logger = mtr::baseline::BaselineLogger::start(
                std::sync::Arc::new(mtr::trippy_engine::TrippyEngine),
                targets,
                sink,
            );
            app.manage(commands::BaselineState(std::sync::Mutex::new(Some(logger))));

            let read_db = std::sync::Arc::new(std::sync::Mutex::new(db::Db::open(&db_path)?));
            app.manage(commands::DbHandle(read_db));

            tray::build_tray(app)?;

            Ok(())
        })
        .manage(commands::SessionState(std::sync::Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            commands::start_session,
            commands::stop_session,
            commands::start_baseline,
            commands::stop_baseline,
            commands::list_candidate_processes,
            commands::list_connections_for_pid,
            commands::get_baseline_history,
            commands::get_session_history,
            commands::export_session_report,
            commands::export_trend_report
        ])
        .run(tauri::generate_context!())
        .expect("error while running Packet Proof");
}
