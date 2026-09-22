#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod db;
mod db_writer;
mod elevation;
mod mtr;
mod procnet;

fn main() {
    if elevation::decide_relaunch(&elevation::check_elevation()) {
        elevation::relaunch_elevated();
    }

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
            commands::get_session_history
        ])
        .run(tauri::generate_context!())
        .expect("error while running Packet Proof");
}
