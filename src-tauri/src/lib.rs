mod audio;
mod commands;
mod autostart;
mod deliver;
mod hotkey;
mod hotkey_ptt; // Enabled - safe FFI with error handling
// mod hotkey_tracker; // Disabled - broken FFI
// mod modifier_hotkey; // Disabled - broken FFI
mod perm;
mod openai;
mod settings;
mod storage;
mod hud;

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, Runtime, WindowEvent};
use crate::storage::{RecordingEntry, Storage};
use crate::settings::{Settings, SettingsStore};
use crate::hotkey::HotkeyState;

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

fn default_hotkey() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Cmd+Shift+Space"
    }
    #[cfg(target_os = "windows")]
    {
        "Ctrl+Shift+Space"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "Ctrl+Shift+Space"
    }
}

fn fallback_hotkey() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Cmd+Option+Space"
    }
    #[cfg(target_os = "windows")]
    {
        "Ctrl+Alt+Space"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "Ctrl+Alt+Space"
    }
}

/// Command to resize HUD window (for auto-sizing based on content)
#[tauri::command]
fn resize_hud_window(app: AppHandle, width: f64, height: f64) -> Result<bool, String> {
    use tauri::Manager;
    
    const MIN_WIDTH: f64 = 40.0;
    const MIN_HEIGHT: f64 = 20.0;
    const MAX_WIDTH: f64 = 2000.0;
    const MAX_HEIGHT: f64 = 1000.0;
    
    let clamped_width = width.max(MIN_WIDTH).min(MAX_WIDTH);
    let clamped_height = height.max(MIN_HEIGHT).min(MAX_HEIGHT);
    
    if let Some(hud_w) = app.get_webview_window(crate::hud::HUD_WINDOW_LABEL) {
        let _ = hud_w.set_size(tauri::Size::Logical(tauri::LogicalSize {
            width: clamped_width,
            height: clamped_height,
        }));
        Ok(true)
    } else {
        Err("HUD window not found".to_string())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            // Create HUD window (global always-on-top), hidden by default.
            // Loads the same frontend with hash #hud to render minimal HUD UI.
            {
                use tauri::{WebviewUrl, WebviewWindowBuilder};
                let hud_url = WebviewUrl::App("index.html#hud".into());
                let hud = WebviewWindowBuilder::new(app, crate::hud::HUD_WINDOW_LABEL, hud_url)
                    .title("Kiklet HUD")
                    .build();
                if let Ok(hud_w) = hud {
                    // Configure HUD window: always-on-top, no decorations, hidden by default.
                    // Transparency is handled via CSS (body background transparent).
                    let _ = hud_w.set_decorations(false);
                    let _ = hud_w.set_always_on_top(true);
                    let _ = hud_w.set_skip_taskbar(true);
                    let _ = hud_w.set_resizable(false);
                    // Initial minimal size - will be auto-resized based on content
                    let _ = hud_w.set_size(tauri::Size::Logical(tauri::LogicalSize { width: 1.0, height: 1.0 }));
                    let _ = hud_w.hide();

                    // Click-through + all spaces (best-effort; macOS-only methods may not exist on other OS).
                    let _ = hud_w.set_ignore_cursor_events(true);
                    #[cfg(target_os = "macos")]
                    {
                        let _ = hud_w.set_visible_on_all_workspaces(true);
                        
                        // Make HUD window fully transparent and add neon pink border (native macOS level)
                        use tauri::Manager;
                        if let Ok(ns_window) = hud_w.ns_window() {
                            unsafe {
                                use objc::{msg_send, sel, sel_impl};
                                use objc::runtime::Object;
                                
                                let window_ptr = ns_window as *mut Object;
                                
                                // Make window background transparent
                                let opaque: bool = false;
                                let _: () = msg_send![window_ptr, setOpaque: opaque];
                                
                                let ns_color_class = objc::runtime::Class::get("NSColor").unwrap();
                                let clear_color: *mut Object = msg_send![ns_color_class, clearColor];
                                let _: () = msg_send![window_ptr, setBackgroundColor: clear_color];
                                
                                // Disable window shadow
                                let has_shadow: bool = false;
                                let _: () = msg_send![window_ptr, setHasShadow: has_shadow];
                                
                                // Get content view and make it transparent
                                let content_view: *mut Object = msg_send![window_ptr, contentView];
                                if !content_view.is_null() {
                                    // Enable layer backing for the content view
                                    let _: () = msg_send![content_view, setWantsLayer: true];
                                    
                                    // Get the layer and make it transparent
                                    let layer: *mut Object = msg_send![content_view, layer];
                                    if !layer.is_null() {
                                        // Set content view layer background to clear
                                        let clear_cg_color: *mut Object = msg_send![clear_color, CGColor];
                                        let _: () = msg_send![layer, setBackgroundColor: clear_cg_color];
                                        
                                        // Create neon pink color (#ff00ff = RGB 255, 0, 255) for border
                                        let red: f64 = 1.0;   // 255/255
                                        let green: f64 = 0.0; // 0/255
                                        let blue: f64 = 1.0;  // 255/255
                                        let alpha: f64 = 1.0; // fully opaque
                                        let neon_pink_color: *mut Object = msg_send![ns_color_class, colorWithRed: red green: green blue: blue alpha: alpha];
                                        
                                        // Get CGColor from NSColor for the layer border
                                        let cg_color: *mut Object = msg_send![neon_pink_color, CGColor];
                                        
                                        // Set border width (2px)
                                        let border_width: f64 = 0.0;
                                        let _: () = msg_send![layer, setBorderWidth: border_width];
                                        
                                        // Set border color
                                        let _: () = msg_send![layer, setBorderColor: cg_color];
                                    }
                                    
                                    // Find WKWebView recursively and make it transparent
                                    let wk_webview_class = objc::runtime::Class::get("WKWebView");
                                    if let Some(wk_class) = wk_webview_class {
                                        // Helper function to find WKWebView recursively
                                        fn find_webview(view: *mut Object, target_class: &objc::runtime::Class) -> Option<*mut Object> {
                                            unsafe {
                                                use objc::{msg_send, sel, sel_impl};
                                                use objc::runtime::Object;
                                                
                                                // Check if this view is WKWebView (using isKindOfClass)
                                                let is_kind: bool = msg_send![view, isKindOfClass: target_class];
                                                if is_kind {
                                                    return Some(view);
                                                }
                                                
                                                // Check subviews recursively
                                                let subviews: *mut Object = msg_send![view, subviews];
                                                if !subviews.is_null() {
                                                    let ns_array: &Object = &*(subviews as *const Object);
                                                    let count: usize = msg_send![ns_array, count];
                                                    for i in 0..count {
                                                        let subview: *mut Object = msg_send![ns_array, objectAtIndex: i];
                                                        if let Some(found) = find_webview(subview, target_class) {
                                                            return Some(found);
                                                        }
                                                    }
                                                }
                                            }
                                            None
                                        }
                                        
                                        if let Some(webview) = find_webview(content_view, &wk_class) {
                                            // Set drawsBackground = false via KVC
                                            let ns_string_class = objc::runtime::Class::get("NSString").unwrap();
                                            let key_cstr = b"drawsBackground\0";
                                            let key_str: *mut Object = msg_send![ns_string_class, stringWithUTF8String: key_cstr.as_ptr()];
                                            // Use setValue:forKey: with NSNumber for boolean
                                            let ns_number_class = objc::runtime::Class::get("NSNumber").unwrap();
                                            let false_number: *mut Object = msg_send![ns_number_class, numberWithBool: false];
                                            let _: () = msg_send![webview, setValue: false_number forKey: key_str];
                                            
                                            // Set opaque = false
                                            let webview_opaque: bool = false;
                                            let _: () = msg_send![webview, setOpaque: webview_opaque];
                                            
                                            // Get webview's layer and make it transparent
                                            let _: () = msg_send![webview, setWantsLayer: true];
                                            let webview_layer: *mut Object = msg_send![webview, layer];
                                            if !webview_layer.is_null() {
                                                let clear_cg_color: *mut Object = msg_send![clear_color, CGColor];
                                                let _: () = msg_send![webview_layer, setBackgroundColor: clear_cg_color];
                                            }
                                            
                                            eprintln!("[kiklet][hud] WKWebView background disabled");
                                        }
                                    }
                                }
                                
                                eprintln!("[kiklet][hud] HUD window: fully transparent (NSWindow + contentView + WKWebView) + neon pink border");
                            }
                        }
                    }
                } else {
                    eprintln!("[kiklet][hud] failed to create hud window");
                }
            }

            let storage = Storage::new(app.handle())?;
            let recordings = storage.load_or_rebuild_index()?;
            let settings_store = SettingsStore::new(app.handle())?;
            let settings = settings_store.load()?;

            app.manage(HotkeyState::default());
            app.manage(AppState {
                storage,
                settings_store,
                settings: Mutex::new(settings),
                recordings: Mutex::new(recordings),
                active_recording: Mutex::new(None),
            });

            setup_tray(app.handle())?;
            setup_close_to_hide(app.handle());

            // Auto-purge old recordings on startup and schedule daily purge
            let app_handle = app.handle().clone();
            
            // Purge on startup (non-blocking)
            {
                let app_handle_purge = app_handle.clone();
                std::thread::spawn(move || {
                    eprintln!("[kiklet] startup: purging old recordings (30 days)");
                    if let Some(state) = app_handle_purge.try_state::<AppState>() {
                        match state.storage.purge_old_recordings(&app_handle_purge, 30) {
                            Ok((deleted, kept)) => {
                                eprintln!("[kiklet] startup purge: deleted={}, kept={}", deleted, kept);
                            }
                            Err(e) => {
                                eprintln!("[kiklet] startup purge failed: {}", e);
                            }
                        }
                    }
                });
            }
            
            // Schedule daily purge (24 hours = 86400 seconds)
            let app_handle_daily = app_handle.clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(86400)); // 24 hours
                    eprintln!("[kiklet] daily: purging old recordings (30 days)");
                    if let Some(state) = app_handle_daily.try_state::<AppState>() {
                        match state.storage.purge_old_recordings(&app_handle_daily, 30) {
                            Ok((deleted, kept)) => {
                                eprintln!("[kiklet] daily purge: deleted={}, kept={}", deleted, kept);
                                // Emit event to refresh UI if window is open
                                let _ = app_handle_daily.emit("recordings_updated", ());
                            }
                            Err(e) => {
                                eprintln!("[kiklet] daily purge failed: {}", e);
                            }
                        }
                    }
                }
            });

            // Register hotkey from settings (or default).
            let state = app.state::<AppState>();
            let s = state.settings.lock().map_err(|_| "settings mutex poisoned")?;
            
            // Temporarily: only support combo hotkeys via plugin
            // Modifier-only and tracker disabled until FFI is fixed
            if s.hotkey_kind == "modifier" && !s.hotkey_accelerator.trim().is_empty() {
                eprintln!("[kiklet][hotkey] modifier-only not supported yet, falling back to default");
                let def = default_hotkey().to_string();
                drop(s);
                if let Err(err) = crate::hotkey::register(app.handle(), &def) {
                    eprintln!("[kiklet][hotkey] error: {err}");
                }
            } else {
                // Combo hotkey (or default)
                let configured = if s.hotkey_accelerator.trim().is_empty() {
                    default_hotkey().to_string()
                } else {
                    s.hotkey_accelerator.trim().to_string()
                };
                
                drop(s);
                
                eprintln!("[kiklet][hotkey] register: {configured}");
                if let Err(err) = crate::hotkey::register(app.handle(), &configured) {
                    eprintln!("[kiklet][hotkey] error: {err}");
                    let fb = fallback_hotkey();
                    eprintln!("[kiklet][hotkey] fallback register: {fb}");
                    if let Err(err2) = crate::hotkey::register(app.handle(), fb) {
                        eprintln!("[kiklet][hotkey] error: {err2}");
                        crate::hotkey::set_error(app.handle(), err2);
                    }
                }
                
                // PTT tracker disabled - FFI causes SIGTRAP
            }

            debug_log("ready");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::set_openai_api_key,
            commands::debug_settings_path,
            commands::get_autostart_status,
            commands::set_autostart_enabled,
            commands::set_autoinsert_enabled,
            commands::deliver_text,
            commands::open_permissions_settings,
            commands::check_permissions,
            commands::request_accessibility,
            commands::get_hotkey,
            commands::set_hotkey,
            commands::reset_hotkey,
            commands::hotkey_status,
            commands::transcribe_file,
            commands::delete_recording,
            commands::start_recording,
            commands::stop_recording,
            commands::list_recordings,
            commands::ptt_start,
            commands::ptt_stop,
            commands::ptt_status,
            commands::set_ptt_threshold_ms,
            commands::list_models,
            commands::translate_text,
            commands::set_translate_target,
            commands::set_translate_model,
            commands::reveal_in_finder,
            commands::open_recordings_folder,
            commands::purge_old_recordings,
            commands::clear_all_recordings,
            commands::debug_dump_storage_paths,
            commands::debug_ping,
            commands::hud_activate,
            commands::hud_deactivate,
            resize_hud_window,
            #[cfg(debug_assertions)]
            commands::debug_print_recordings_paths,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
