// Muivly settings panel. Renders no wallpaper — that is the engine's job, in
// its own process, so this window's WebView memory is a cost paid only while
// the settings are open.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod engine;
mod pack;
mod pipe;
mod shell;
mod steam;
mod store;
mod web;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WindowEvent};

fn main() {
    // `--set <file>` is Explorer's right-click menu, not a person opening the
    // app: hand the path to the engine and exit. Checked before anything
    // Tauri does, because the point of that path is that it costs no window
    // and no WebView — a wallpaper change should not open an application.
    if shell::handle_cli() {
        return;
    }

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
            pipe::set_visual,
            pipe::set_sound,
            pipe::set_power,
            pipe::set_speed,
            pipe::set_fade,
            pipe::set_span,
            pipe::set_hotkeys,
            pipe::set_frozen,
            pipe::set_overrides,
            pipe::quit_engine,
            shell::context_menu_enabled,
            shell::set_context_menu,
            pack::export_package,
            pack::import_package,
            pack::inspect_package,
            engine::engine_installed,
            engine::start_engine,
            autostart::autostart_enabled,
            autostart::set_autostart,
            steam::scan_wallpaper_engine,
            web::web_fetch,
            web::web_download,
            web::wallpapers_path,
            store::load_state,
            store::save_state,
            store::state_path,
            store::file_exists,
            store::file_infos,
            store::reveal,
        ])
        .setup(|app| {
            // The menu a wallpaper app is actually used through. Opening the
            // settings window to skip to the next wallpaper or to quieten it
            // is three clicks and a WebView; from here it is one click.
            let show = MenuItem::with_id(app, "show", "Muivly'yi aç", true, None::<&str>)?;
            // The shortcuts are named in the labels because a tray menu is
            // where people find out they exist.
            let next = MenuItem::with_id(
                app,
                "next",
                "Sonraki duvar kağıdı\tCtrl+Alt+→",
                true,
                None::<&str>,
            )?;
            let freeze = MenuItem::with_id(
                app,
                "freeze",
                "Dondur / devam et\tCtrl+Alt+P",
                true,
                None::<&str>,
            )?;
            let pause = MenuItem::with_id(app, "pause", "Duraklat / devam et", true, None::<&str>)?;
            let mute = MenuItem::with_id(
                app,
                "mute",
                "Sesi aç / kapat\tCtrl+Alt+M",
                true,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "Çıkış", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &show,
                    &PredefinedMenuItem::separator(app)?,
                    &next,
                    &freeze,
                    &pause,
                    &mute,
                    &PredefinedMenuItem::separator(app)?,
                    &quit,
                ],
            )?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Muivly")
                .menu(&menu)
                // Left click is handled below; without this the menu would
                // also open on left click and swallow the click.
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => reveal(app),
                    "next" => pipe::next_all(),
                    "freeze" => pipe::toggle_freeze(),
                    "pause" => pipe::toggle_all(),
                    "mute" => pipe::toggle_sound(),
                    "quit" => shut_down(app),
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

/// Leave, and take the wallpaper with us.
///
/// Closing the settings window only hides it, because the engine is meant to
/// keep running without the panel open. "Çıkış" is the other thing: the user
/// is done with Muivly, and an engine still drawing to their desktop after
/// they quit is not a background service, it is a process they cannot get
/// rid of. Failure is ignored on purpose — the usual reason is that the
/// engine was not running in the first place.
fn shut_down(app: &tauri::AppHandle) {
    let _ = pipe::quit_engine();
    app.exit(0);
}
