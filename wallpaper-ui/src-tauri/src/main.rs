// Muivly settings panel. Renders no wallpaper — that is the engine's job, in
// its own process, so this window's WebView memory is a cost paid only while
// the settings are open.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engine;
mod pipe;
mod store;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WindowEvent};

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            pipe::status,
            pipe::monitors,
            pipe::set_playlist,
            pipe::next_item,
            pipe::set_monitor_enabled,
            pipe::set_fps,
            pipe::set_fit,
            pipe::set_interval,
            pipe::quit_engine,
            engine::engine_installed,
            engine::start_engine,
            store::load_state,
            store::save_state,
            store::state_path,
            store::file_exists,
        ])
        .setup(|app| {
            let show = MenuItem::with_id(app, "show", "Muivly'yi aç", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Çıkış", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Muivly")
                .menu(&menu)
                // Left click is handled below; without this the menu would
                // also open on left click and swallow the click.
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => reveal(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        reveal(tray.app_handle());
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window hides it instead. The wallpaper keeps
            // running either way — it is a different process — but a user
            // who clicks X expects the app to still be there, the way every
            // other tray application behaves.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to start Muivly");
}

/// Bring the settings window back from the tray.
fn reveal(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        // A window that was minimised stays minimised after show().
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
