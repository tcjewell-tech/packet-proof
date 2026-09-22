#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod elevation;
mod mtr;

fn main() {
    if elevation::decide_relaunch(&elevation::check_elevation()) {
        elevation::relaunch_elevated();
    }

    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running Packet Proof");
}
