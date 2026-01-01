mod audio;
mod commands;
mod openai;
mod settings;
mod storage;

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, Runtime, WindowEvent};
use crate::storage::{RecordingEntry, Storage};
use crate::settings::{Settings, SettingsStore};

const MAIN_WINDOW_LABEL: &str = "main";
const TRAY_ID: &str = "kiklet-tray";

pub struct AppState {
    pub storage: Storage,
    pub settings_store: SettingsStore,
    pub settings: Mutex<Settings>,
    pub recordings: Mutex<Vec<RecordingEntry>>,
    pub active_recording: Mutex<Option<audio::RecordingSession>>,
}

fn debug_log(msg: &str) {
    if cfg!(debug_assertions) {
        eprintln!("[kiklet] {msg}");
    }
}

pub fn emit_recording_state(app: &AppHandle, is_recording: bool) -> Result<(), tauri::Error> {
    app.emit("recording_state", is_recording)
}

fn show_main_window(app: &AppHandle) -> Result<(), tauri::Error> {
    if let Some(w) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = w.show();
        let _ = w.set_focus();
    }
    if let Some(state) = app.try_state::<AppState>() {
        let is_recording = state
            .active_recording
            .lock()
            .ok()
            .map(|g| g.is_some())
            .unwrap_or(false);
        let _ = emit_recording_state(app, is_recording);
    }
    Ok(())
}

fn build_tray_menu<R: Runtime>(
    app: &tauri::AppHandle<R>,
    is_recording: bool,
) -> Result<tauri::menu::Menu<R>, tauri::Error> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};

    let start = MenuItemBuilder::with_id("start_recording", "Start Recording")
        .enabled(!is_recording)
        .build(app)?;
    let stop = MenuItemBuilder::with_id("stop_recording", "Stop Recording")
        .enabled(is_recording)
        .build(app)?;
    let open = MenuItemBuilder::with_id("open_kiklet", "Open Kiklet").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    MenuBuilder::new(app)
        .items(&[&start, &stop, &open, &quit])
        .build()
}

pub fn set_tray_recording_state(app: &AppHandle, is_recording: bool) -> Result<(), tauri::Error> {
    let menu = build_tray_menu(app, is_recording)?;
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(menu))?;
    }
    Ok(())
}

fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::tray::{TrayIconBuilder, TrayIconEvent};

    let menu = build_tray_menu(app, false)?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("missing default window icon")?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .menu(&menu)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            let handle = app.app_handle();
            match id {
                "start_recording" => {
                    let _ = crate::commands::start_recording(handle.clone(), handle.state());
                }
                "stop_recording" => {
                    let _ = crate::commands::stop_recording(handle.clone(), handle.state());
                }
                "open_kiklet" => {
                    let _ = show_main_window(&handle);
                }
                "quit" => {
                    handle.exit(0);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { .. } = event {
                let handle = tray.app_handle();
                let _ = show_main_window(&handle);
            }
        })
        .build(app)?;

    Ok(())
}

fn setup_close_to_hide(app: &AppHandle) {
    let Some(w) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    let w2 = w.clone();
    w.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = w2.hide();
        }
    });
}

fn setup_hotkey(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    // Keep idle cost at ~0: register once, no background loop.
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    use tauri_plugin_global_shortcut::ShortcutState;

    app.global_shortcut()
        .on_shortcut("Command+Shift+Space", |app_handle, _shortcut, event| {
            // Hold-to-talk:
            // - Pressed: start recording
            // - Released: stop recording
            let state = app_handle.state::<AppState>();
            match event.state {
                ShortcutState::Pressed => {
                    let _ = crate::commands::start_recording(app_handle.clone(), state);
                }
                ShortcutState::Released => {
                    let _ = crate::commands::stop_recording(app_handle.clone(), state);
                }
            }
        })?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let storage = Storage::new(app.handle())?;
            let recordings = storage.load_or_rebuild_index()?;
            let settings_store = SettingsStore::new(app.handle())?;
            let settings = settings_store.load()?;

            app.manage(AppState {
                storage,
                settings_store,
                settings: Mutex::new(settings),
                recordings: Mutex::new(recordings),
                active_recording: Mutex::new(None),
            });

            setup_tray(app.handle())?;
            setup_close_to_hide(app.handle());
            setup_hotkey(app.handle())?;

            debug_log("ready");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::set_openai_api_key,
            commands::debug_settings_path,
            commands::transcribe_file,
            commands::start_recording,
            commands::stop_recording,
            commands::list_recordings,
            commands::reveal_in_finder,
            commands::open_recordings_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
