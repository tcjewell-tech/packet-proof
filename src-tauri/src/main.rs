#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod elevation;
mod mtr;

fn main() {
    if elevation::decide_relaunch(&elevation::check_elevation()) {
        elevation::relaunch_elevated();
    }

    tauri::Builder::default()
        .manage(commands::DemoState(std::sync::Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![commands::start_demo_session])
        .run(tauri::generate_context!())
        .expect("error while running Packet Proof");
}
