//! JARVIS Desktop — Tauri shell: system tray, frameless HUD window,
//! Ctrl+Shift+J global hotkey, single-instance lock, autostart.

mod brain;
mod calendar;
mod claude;
mod db;
mod google_auth;
mod hud;
mod mcp;
mod proactive;
mod secrets;
mod stt;
mod todos;
mod tools;
mod tts;
mod work;

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};
use tauri_plugin_autostart::ManagerExt as AutostartExt;
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutEvent, ShortcutState};

pub struct Flags {
    pub muted: AtomicBool,
}

/// Which summon hotkey actually registered ("" if none was available).
pub struct ActiveHotkey(pub String);

fn on_hotkey(app: &AppHandle, _sc: &Shortcut, event: ShortcutEvent) {
    match event.state() {
        ShortcutState::Pressed => {
            show_main(app);
            // The UI starts push-to-talk on this event.
            let _ = app.emit("hotkey-summon", true);
        }
        ShortcutState::Released => {
            let _ = app.emit("hotkey-released", true);
        }
    }
}

#[tauri::command]
fn get_hotkey(app: AppHandle) -> String {
    app.state::<ActiveHotkey>().0.clone()
}

fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
    // First wake of the day → morning briefing.
    proactive::maybe_brief(app);
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

/// Mini reactor widget → click expands the full app.
#[tauri::command]
fn show_main_window(app: AppHandle) {
    show_main(&app);
}

fn set_mini_visible(app: &AppHandle, visible: bool) {
    if let Some(win) = app.get_webview_window("mini") {
        let _ = if visible { win.show() } else { win.hide() };
    }
    let db = app.state::<db::Db>();
    let _ = db::kv_set(&db, "mini_visible", if visible { "1" } else { "0" });
}

#[tauri::command]
fn toggle_mini(app: AppHandle) -> bool {
    let visible = app
        .get_webview_window("mini")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    set_mini_visible(&app, !visible);
    !visible
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
        .manage(brain::Brain::default())
        .manage(stt::SttState::default())
        .manage(tools::Timers::default())
        .manage(proactive::AlertLog::default())
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
            calendar::calendar_connect,
            calendar::calendar_today,
            hud::worker_api,
            hud::worker_mutate,
            tools::weather_now,
            todos::todo_list,
            todos::todo_add,
            todos::todo_complete,
            todos::todo_confirm,
            todos::todo_snooze,
            todos::todo_dismiss,
            work::work_email_overview,
            work::work_slack_overview,
            work::work_calendar_today,
            work::work_google_connect,
            work::work_scan,
            tts::tts_voices,
            tts::tts_synthesize,
            tts::open_voice_settings,
            brain::brain_status,
            brain::brain_set_mode,
            toggle_mute,
            is_muted,
            hide_window,
            show_main_window,
            toggle_mini,
            get_hotkey,
        ])
        .setup(|app| {
            // ---- local SQLite (notes + settings) ----
            let database = db::init(app.handle())?;
            app.manage(database);

            // ---- MCP shim: the CLI brain's bridge to the app's tools ----
            let mcp_info = mcp::start(app.handle());
            *app.state::<brain::Brain>().mcp_url.lock().unwrap() = mcp_info.url;

            // ---- proactive loops: morning briefing + anomaly watch ----
            proactive::start(app.handle());

            // ---- system tray ----
            let open = MenuItem::with_id(app, "open", "Open JARVIS", true, None::<&str>)?;
            let mini_on = {
                let db = app.state::<db::Db>();
                db::kv_get(&db, "mini_visible").as_deref() == Some("1")
            };
            let mini = CheckMenuItem::with_id(app, "mini", "Mini reactor", true, mini_on, None::<&str>)?;
            if mini_on {
                set_mini_visible(app.handle(), true);
            }
            let mute = CheckMenuItem::with_id(app, "mute", "Mute", true, false, None::<&str>)?;
            let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
            let autostart =
                CheckMenuItem::with_id(app, "autostart", "Start with Windows", true, autostart_enabled, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &mini, &mute, &autostart, &quit])?;

            let mute_item = mute.clone();
            let mini_item = mini.clone();
            TrayIconBuilder::with_id("jarvis-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("JARVIS")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "quit" => app.exit(0),
                    "mini" => {
                        let visible = toggle_mini(app.clone());
                        let _ = mini_item.set_checked(visible);
                    }
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

            // ---- global hotkey: Ctrl+Shift+J, falling back to Ctrl+Alt+J ----
            // Registration failure must NEVER abort startup (another app may
            // own the combination) — the tray and window still work without it.
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            let primary = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyJ);
            let fallback = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyJ);
            let gs = app.global_shortcut();
            let active = if gs.on_shortcut(primary, on_hotkey).is_ok() {
                "Ctrl+Shift+J"
            } else if gs.on_shortcut(fallback, on_hotkey).is_ok() {
                eprintln!("JARVIS: Ctrl+Shift+J is taken by another app; using Ctrl+Alt+J");
                "Ctrl+Alt+J"
            } else {
                eprintln!("JARVIS: no global hotkey available; use the tray icon");
                ""
            };
            app.manage(ActiveHotkey(active.into()));

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
