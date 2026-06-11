//! JARVIS Desktop — Tauri shell: system tray, frameless HUD window,
//! Ctrl+Shift+J global hotkey, single-instance lock, autostart.

mod claude;
mod db;
mod secrets;
mod stt;
mod tools;

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};
use tauri_plugin_autostart::ManagerExt as AutostartExt;
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};

pub struct Flags {
    pub muted: AtomicBool,
}

fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

#[tauri::command]
fn toggle_mute(app: AppHandle) -> bool {
    let flags = app.state::<Flags>();
    let muted = !flags.muted.load(Ordering::Relaxed);
    flags.muted.store(muted, Ordering::Relaxed);
    let _ = app.emit("mute-changed", muted);
    muted
}

#[tauri::command]
fn is_muted(app: AppHandle) -> bool {
    app.state::<Flags>().muted.load(Ordering::Relaxed)
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second launch just summons the existing instance.
            show_main(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .manage(claude::Session::default())
        .manage(stt::SttState::default())
        .manage(tools::Timers::default())
        .manage(Flags {
            muted: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            claude::ask_jarvis,
            claude::clear_session,
            secrets::secret_set,
            secrets::secret_exists,
            stt::stt_listen,
            stt::stt_stop,
            db::setting_get,
            db::setting_set,
            toggle_mute,
            is_muted,
            hide_window,
        ])
        .setup(|app| {
            // ---- local SQLite (notes + settings) ----
            let database = db::init(app.handle())?;
            app.manage(database);

            // ---- system tray ----
            let open = MenuItem::with_id(app, "open", "Open JARVIS", true, None::<&str>)?;
            let mute = CheckMenuItem::with_id(app, "mute", "Mute", true, false, None::<&str>)?;
            let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
            let autostart =
                CheckMenuItem::with_id(app, "autostart", "Start with Windows", true, autostart_enabled, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &mute, &autostart, &quit])?;

            let mute_item = mute.clone();
            TrayIconBuilder::with_id("jarvis-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("JARVIS")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "quit" => app.exit(0),
                    "mute" => {
                        let muted = toggle_mute(app.clone());
                        let _ = mute_item.set_checked(muted);
                    }
                    "autostart" => {
                        let launcher = app.autolaunch();
                        let enabled = launcher.is_enabled().unwrap_or(false);
                        let _ = if enabled { launcher.disable() } else { launcher.enable() };
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;

            // ---- global hotkey: Ctrl+Shift+J — summon + start listening ----
            let hotkey = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyJ);
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            app.global_shortcut().on_shortcut(hotkey, |app, _sc, event| {
                match event.state() {
                    ShortcutState::Pressed => {
                        show_main(app);
                        // Stage 2: the UI starts push-to-talk on this event.
                        let _ = app.emit("hotkey-summon", true);
                    }
                    ShortcutState::Released => {
                        let _ = app.emit("hotkey-released", true);
                    }
                }
            })?;

            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window hides to tray; Quit lives in the tray menu.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running JARVIS");
}
