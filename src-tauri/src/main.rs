#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod db;
mod db_writer;
mod elevation;
mod mtr;

fn main() {
    if elevation::decide_relaunch(&elevation::check_elevation()) {
        elevation::relaunch_elevated();
    }

    tauri::Builder::default()
        .setup(|app| {
            use tauri::Manager;
            let db_path = app.path().app_data_dir()?.join("packet_proof.sqlite");
            std::fs::create_dir_all(db_path.parent().unwrap())?;
            let sink = db_writer::spawn_db_writer(db_path);
            app.manage(commands::DbSink(sink));
            Ok(())
        })
        .manage(commands::SessionState(std::sync::Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![commands::start_session, commands::stop_session])
        .run(tauri::generate_context!())
        .expect("error while running Packet Proof");
}
