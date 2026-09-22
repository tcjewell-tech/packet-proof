#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod db;
mod elevation;
mod mtr;

fn main() {
    if elevation::decide_relaunch(&elevation::check_elevation()) {
        elevation::relaunch_elevated();
    }

    tauri::Builder::default()
        .manage(commands::SessionState(std::sync::Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![commands::start_session, commands::stop_session])
        .run(tauri::generate_context!())
        .expect("error while running Packet Proof");
}
