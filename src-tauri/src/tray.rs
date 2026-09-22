use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

pub fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let start_baseline_item = MenuItem::with_id(
        app,
        "start_baseline",
        "Start baseline logger",
        true,
        None::<&str>,
    )?;
    let stop_baseline_item = MenuItem::with_id(
        app,
        "stop_baseline",
        "Stop baseline logger",
        true,
        None::<&str>,
    )?;
    let new_session_item = MenuItem::with_id(
        app,
        "new_session",
        "New timed session…",
        true,
        None::<&str>,
    )?;
    let show_item = MenuItem::with_id(app, "show", "Show dashboard", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &start_baseline_item,
            &stop_baseline_item,
            &new_session_item,
            &show_item,
            &quit_item,
        ],
    )?;

    let mut builder = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Packet Proof — elevated, baseline running");

    // `TrayIconBuilder` does not fall back to the app's default window icon
    // the way window creation does, so it must be set explicitly here or
    // the tray icon renders blank/invisible on Windows.
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder
        .on_menu_event(|app: &AppHandle<Wry>, event| match event.id().as_ref() {
            "start_baseline" => {
                let state = app.state::<crate::commands::BaselineState>();
                let db_sink = app.state::<crate::commands::DbSink>();
                let _ = crate::commands::start_baseline(state, db_sink);
            }
            "stop_baseline" => {
                let state = app.state::<crate::commands::BaselineState>();
                let _ = crate::commands::stop_baseline(state);
            }
            "new_session" | "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}
