use tauri::{AppHandle, Emitter, Manager, State};

use crate::audio;
use crate::autostart;
use crate::deliver;
use crate::hotkey;
use crate::hotkey_ptt;
// use crate::hotkey_tracker; // Disabled - broken FFI
// use crate::modifier_hotkey; // Disabled - broken FFI
use crate::openai;
use crate::perm;
use crate::storage::{recordings_dir, ClearAllReport, RecordingEntry, RecordingsIndex};
use crate::{emit_recording_state, set_tray_recording_state, AppState};

fn trunc200(s: &str) -> String {
    let mut out = s.replace('\n', " ").replace('\r', " ");
    if out.len() > 200 {
        out.truncate(200);
    }
    out
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub openai_api_key: String,
    pub has_openai_api_key: bool,
    pub autoinsert_enabled: bool,
    pub hotkey_accelerator: String,
    pub hotkey_code: Option<String>,
    pub hotkey_mods: Option<crate::settings::HotkeyMods>,
    pub ptt_enabled: bool,
    pub ptt_threshold_ms: u64,
    pub translate_target: Option<String>,
    pub translate_model: String,
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<SettingsDto, String> {
    let s = state
        .settings
        .lock()
        .map_err(|_| "settings mutex poisoned".to_string())?
        .clone();
    Ok(SettingsDto {
        has_openai_api_key: !s.openai_api_key.trim().is_empty(),
        openai_api_key: s.openai_api_key,
        autoinsert_enabled: s.autoinsert_enabled,
        hotkey_accelerator: s.hotkey_accelerator,
        hotkey_code: s.hotkey_code,
        hotkey_mods: s.hotkey_mods,
        ptt_enabled: s.ptt_enabled,
        ptt_threshold_ms: s.ptt_threshold_ms,
        translate_target: s.translate_target.clone(),
        translate_model: s.translate_model.clone(),
    })
}

#[tauri::command]
pub fn set_openai_api_key(state: State<'_, AppState>, api_key: String) -> Result<(), String> {
    eprintln!(
        "[kiklet][settings] set_openai_api_key called (len={})",
        api_key.len()
    );
    let trimmed = api_key.trim().to_string();
    {
        let mut s = state
            .settings
            .lock()
            .map_err(|_| "settings mutex poisoned".to_string())?;
        s.openai_api_key = trimmed;
        if let Err(err) = state.settings_store.save(&s) {
            eprintln!("[kiklet][settings] save failed: {err}");
            return Err(err.to_string());
        }
        eprintln!(
            "[kiklet][settings] saved to {}",
            state.settings_store.path.display()
        );
        eprintln!("[kiklet][settings] saved ok");
    }
    Ok(())
}

#[tauri::command]
pub fn debug_settings_path(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.settings_store.path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn set_autoinsert_enabled(state: State<'_, AppState>, enabled: bool) -> Result<bool, String> {
    let mut s = state
        .settings
        .lock()
        .map_err(|_| "settings mutex poisoned".to_string())?;
    s.autoinsert_enabled = enabled;
    if let Err(err) = state.settings_store.save(&s) {
        return Err(err.to_string());
    }
    Ok(s.autoinsert_enabled)
}

#[tauri::command]
pub fn deliver_text(_state: State<'_, AppState>, text: String) -> Result<deliver::DeliveryResult, String> {
    #[cfg(target_os = "macos")]
    let platform = "macos";
    #[cfg(target_os = "windows")]
    let platform = "windows";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let platform = "other";

    let attempt_insert = {
        let s = _state
            .settings
            .lock()
            .map_err(|_| "settings mutex poisoned".to_string())?;
        s.autoinsert_enabled
    };

    eprintln!(
        "[kiklet][deliver] start platform={} len={} attempt_insert={}",
        platform,
        text.len(),
        attempt_insert
    );
    match deliver::deliver(text.as_str(), attempt_insert) {
        Ok(res) => {
            let detail_str = res.detail.clone().unwrap_or_else(|| "-".to_string());
            eprintln!(
                "[kiklet][deliver] ok platform={} len={} mode={:?} detail={}",
                platform,
                text.len(),
                res.mode,
                detail_str
            );
            Ok(res)
        }
        Err(err) => {
            eprintln!(
                "[kiklet][deliver] error platform={} len={} err='{}'",
                platform,
                text.len(),
                trunc200(&err)
            );
            Err(err)
        }
    }
}

#[tauri::command]
pub fn open_permissions_settings() -> Result<(), String> {
    eprintln!("[kiklet][deliver] open_permissions_settings");
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .output()
            .map_err(|e| format!("failed to run open: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        return Err(format!(
            "open failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    #[cfg(target_os = "windows")]
    {
        // `start` needs a title argument.
        let out = std::process::Command::new("cmd")
            .args(["/C", "start", "", "ms-settings:privacy-clipboard"])
            .output()
            .map_err(|e| format!("failed to run cmd: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        let out2 = std::process::Command::new("cmd")
            .args(["/C", "start", "", "ms-settings:privacy"])
            .output()
            .map_err(|e| format!("failed to run cmd: {e}"))?;
        if out2.status.success() {
            return Ok(());
        }
        return Err(format!(
            "failed to open settings: {}",
            String::from_utf8_lossy(&out2.stderr)
        ));
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err("unsupported platform".to_string())
    }
}

#[tauri::command]
pub fn check_permissions() -> Result<perm::PermissionCheckResult, String> {
    #[cfg(target_os = "macos")]
    let platform = "macos";
    #[cfg(target_os = "windows")]
    let platform = "windows";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let platform = "other";

    let res = perm::check_permissions()?;
    eprintln!(
        "[kiklet][perm] platform={} ax_ok={} need_accessibility={}",
        platform,
        res.ok,
        res.need_accessibility
    );
    Ok(res)
}

#[tauri::command]
pub fn request_accessibility() -> Result<perm::RequestAccessibilityResult, String> {
    perm::request_accessibility()
}

#[tauri::command]
pub fn get_autostart_status(app: tauri::AppHandle) -> Result<bool, String> {
    let enabled = autostart::is_enabled(&app)?;
    // sync settings silently to reality
    let state = app.state::<AppState>();
    {
        let mut s = state
            .settings
            .lock()
            .map_err(|_| "settings mutex poisoned".to_string())?;
        if s.autostart_enabled != enabled {
            s.autostart_enabled = enabled;
            if let Err(err) = state.settings_store.save(&s) {
                eprintln!("[kiklet][autostart] error: failed to sync settings: {err}");
            }
        }
    }
    Ok(enabled)
}

#[tauri::command]
pub fn set_autostart_enabled(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    let state = app.state::<AppState>();

    if enabled {
        if let Err(err) = autostart::enable(&app) {
            eprintln!("[kiklet][autostart] error: {err}");
            return Err(err);
        }
    } else {
        if let Err(err) = autostart::disable(&app) {
            eprintln!("[kiklet][autostart] error: {err}");
            return Err(err);
        }
    }

    let actual = autostart::is_enabled(&app)?;
    {
        let mut s = state
            .settings
            .lock()
            .map_err(|_| "settings mutex poisoned".to_string())?;
        s.autostart_enabled = actual;
        if let Err(err) = state.settings_store.save(&s) {
            eprintln!("[kiklet][autostart] error: failed to save settings: {err}");
            return Err(err.to_string());
        }
    }
    Ok(actual)
}

fn default_hotkey_str() -> &'static str {
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

fn fallback_hotkey_str() -> &'static str {
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

fn validate_hotkey(acc: &str) -> Result<(), String> {
    let a = acc.trim();
    if a.is_empty() {
        return Err("hotkey is empty".to_string());
    }
    if a.len() > 64 {
        return Err("hotkey is too long".to_string());
    }
    
    let lower = a.to_lowercase();
    
    // Check for pure modifiers (forbidden standalone)
    let pure_modifiers = [
        "shift", "ctrl", "control", "alt", "cmd", "command", "option",
        "win", "super", "meta",
    ];
    for mod_key in &pure_modifiers {
        if lower == *mod_key {
            return Err("hotkey must include a non-modifier key".to_string());
        }
    }
    
    // Check if has modifiers (then it's valid)
    let has_mod = lower.contains("ctrl")
        || lower.contains("alt")
        || lower.contains("shift")
        || lower.contains("cmd")
        || lower.contains("command")
        || lower.contains("option")
        || lower.contains("win")
        || lower.contains("super")
        || lower.contains("meta");
    if has_mod {
        return Ok(());
    }
    
    // Allow standalone special keys (no modifiers required)
    let allowed_standalone = [
        "ins", "insert",
        "pageup", "pagedown",
        "home", "end",
        "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10",
        "f11", "f12", "f13", "f14", "f15", "f16", "f17", "f18", "f19",
        "f20", "f21", "f22", "f23", "f24",
        "pause", "printscreen", "scrolllock",
        "arrowup", "arrowdown", "arrowleft", "arrowright",
        "tab", "space", "enter",
    ];
    for allowed in &allowed_standalone {
        if lower == *allowed {
            return Ok(());
        }
    }
    
    // Allow single character keys (A-Z, 0-9, symbols) as standalone
    if a.len() == 1 {
        return Ok(());
    }
    
    Err("hotkey must include a non-modifier key or be a valid standalone key".to_string())
}

#[tauri::command]
pub fn hotkey_status(app: tauri::AppHandle) -> Result<bool, String> {
    Ok(hotkey::is_enabled(&app))
}

#[tauri::command]
pub fn get_hotkey(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let s = state
        .settings
        .lock()
        .map_err(|_| "settings mutex poisoned".to_string())?;
    let v = s.hotkey_accelerator.trim();
    if v.is_empty() {
        Ok(default_hotkey_str().to_string())
    } else {
        Ok(v.to_string())
    }
}

// Generate candidates from code-based accelerator (for combo)
// Generate plugin-compatible key name candidates from physical code
fn plugin_key_candidates_from_code(code: &str) -> Vec<String> {
    let candidates: Vec<&str> = match code {
        // Digits
        "Digit1" | "1" => vec!["1", "Digit1", "Key1", "Numpad1"],
        "Digit2" | "2" => vec!["2", "Digit2", "Key2", "Numpad2"],
        "Digit3" | "3" => vec!["3", "Digit3", "Key3", "Numpad3"],
        "Digit4" | "4" => vec!["4", "Digit4", "Key4", "Numpad4"],
        "Digit5" | "5" => vec!["5", "Digit5", "Key5", "Numpad5"],
        "Digit6" | "6" => vec!["6", "Digit6", "Key6", "Numpad6"],
        "Digit7" | "7" => vec!["7", "Digit7", "Key7", "Numpad7"],
        "Digit8" | "8" => vec!["8", "Digit8", "Key8", "Numpad8"],
        "Digit9" | "9" => vec!["9", "Digit9", "Key9", "Numpad9"],
        "Digit0" | "0" => vec!["0", "Digit0", "Key0", "Numpad0"],
        // Letters (only if code starts with "Key")
        "KeyQ" => vec!["Q", "KeyQ"],
        "KeyW" => vec!["W", "KeyW"],
        "KeyE" => vec!["E", "KeyE"],
        "KeyR" => vec!["R", "KeyR"],
        "KeyT" => vec!["T", "KeyT"],
        "KeyY" => vec!["Y", "KeyY"],
        "KeyU" => vec!["U", "KeyU"],
        "KeyI" => vec!["I", "KeyI"],
        "KeyO" => vec!["O", "KeyO"],
        "KeyP" => vec!["P", "KeyP"],
        "KeyA" => vec!["A", "KeyA"],
        "KeyS" => vec!["S", "KeyS"],
        "KeyD" => vec!["D", "KeyD"],
        "KeyF" => vec!["F", "KeyF"],
        "KeyG" => vec!["G", "KeyG"],
        "KeyH" => vec!["H", "KeyH"],
        "KeyJ" => vec!["J", "KeyJ"],
        "KeyK" => vec!["K", "KeyK"],
        "KeyL" => vec!["L", "KeyL"],
        "KeyZ" => vec!["Z", "KeyZ"],
        "KeyX" => vec!["X", "KeyX"],
        "KeyC" => vec!["C", "KeyC"],
        "KeyV" => vec!["V", "KeyV"],
        "KeyB" => vec!["B", "KeyB"],
        "KeyN" => vec!["N", "KeyN"],
        "KeyM" => vec!["M", "KeyM"],
        // Special keys
        "Space" => vec!["Space"],
        "Insert" => vec!["Insert", "Ins"],
        "PageUp" => vec!["PageUp", "Prior"],
        "PageDown" => vec!["PageDown", "Next"],
        "Home" => vec!["Home"],
        "End" => vec!["End"],
        "ArrowUp" => vec!["ArrowUp", "Up"],
        "ArrowDown" => vec!["ArrowDown", "Down"],
        "ArrowLeft" => vec!["ArrowLeft", "Left"],
        "ArrowRight" => vec!["ArrowRight", "Right"],
        "F1" => vec!["F1"],
        "F2" => vec!["F2"],
        "F3" => vec!["F3"],
        "F4" => vec!["F4"],
        "F5" => vec!["F5"],
        "F6" => vec!["F6"],
        "F7" => vec!["F7"],
        "F8" => vec!["F8"],
        "F9" => vec!["F9"],
        "F10" => vec!["F10"],
        "F11" => vec!["F11"],
        "F12" => vec!["F12"],
        "Tab" => vec!["Tab"],
        "Enter" => vec!["Enter"],
        "Escape" => vec!["Escape"],
        "Backspace" => vec!["Backspace"],
        "Delete" => vec!["Delete"],
        // Fallback: use code as-is only if it looks valid
        _ => {
            // Only accept if it's a known format (starts with Key/Digit/Arrow/etc)
            if code.starts_with("Key") || code.starts_with("Digit") || code.starts_with("Arrow") 
                || code.starts_with("F") || code == "Space" || code == "Insert" || code == "PageUp" 
                || code == "PageDown" || code == "Home" || code == "End" || code == "Tab" 
                || code == "Enter" || code == "Escape" || code == "Backspace" || code == "Delete" {
                vec![code]
            } else {
                // Invalid code - return empty to fail registration
                vec![]
            }
        }
    };
    candidates.into_iter().map(|s| s.to_string()).collect()
}

fn generate_hotkey_candidates_from_code(code: &str, mods: &crate::settings::HotkeyMods) -> Vec<String> {
    let mut candidates = vec![];
    
    // Build modifier prefix
    let mut mod_parts = vec![];
    if mods.cmd {
        mod_parts.push("Cmd");
    }
    if mods.ctrl {
        mod_parts.push("Ctrl");
    }
    if mods.alt {
        mod_parts.push("Alt");
    }
    if mods.shift {
        mod_parts.push("Shift");
    }
    
    // Get plugin-compatible key name candidates
    let key_candidates = plugin_key_candidates_from_code(code);
    
    if key_candidates.is_empty() {
        // Invalid code - return empty to fail
        return vec![];
    }
    
    // Build candidates with modifiers
    if mod_parts.is_empty() {
        // No modifiers: standalone key
        candidates.extend(key_candidates);
    } else {
        // With modifiers
        let mod_prefix = mod_parts.join("+");
        for key in key_candidates {
            candidates.push(format!("{}+{}", mod_prefix, key));
        }
    }
    
    // Remove duplicates
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|c| seen.insert(c.clone()));
    
    candidates
}

#[tauri::command]
pub fn set_hotkey(
    app: tauri::AppHandle,
    accelerator: String,
    kind: String,
    code: Option<String>,
    mods: Option<crate::settings::HotkeyMods>,
) -> Result<String, String> {
    let acc = accelerator.trim().to_string();
    
    match kind.as_str() {
        "modifier" => {
            // Modifier-only: not supported yet (FFI broken)
            return Err("modifier-only hotkeys not supported yet".to_string());
        }
        "combo" => {
            // Combo: register via plugin only (PTT disabled - FFI causes SIGTRAP)
            // Get code and mods
            let code_str = code.ok_or_else(|| "missing code for combo".to_string())?;
            let mods_val = mods.ok_or_else(|| "missing mods for combo".to_string())?;
            
            // Generate candidates from code
            let candidates = generate_hotkey_candidates_from_code(&code_str, &mods_val);
            
            eprintln!("[kiklet][hotkey] kind=combo attempt register {} ({} candidates)", acc, candidates.len());
            
            // Try each candidate
            let mut last_err: Option<String> = None;
            let mut registered: Option<String> = None;
            
            for cand in &candidates {
                eprintln!("[kiklet][hotkey] kind=combo try_register {}", cand);
                
                match hotkey::register(&app, cand) {
                    Ok(()) => {
                        eprintln!("[kiklet][hotkey] register_ok {}", cand);
                        registered = Some(cand.clone());
                        break;
                    }
                    Err(err) => {
                        eprintln!("[kiklet][hotkey] register_failed {}: {}", cand, err);
                        last_err = Some(err);
                    }
                }
            }
            
            // If none succeeded, return error
            let normalized = match registered {
                Some(norm) => norm,
                None => {
                    let err_msg = last_err.unwrap_or_else(|| "unknown error".to_string());
                    return Err(format!(
                        "failed to register hotkey: {} (tried: {})",
                        err_msg,
                        candidates.join(", ")
                    ));
                }
            };
            
            // Success: save to settings
            let state = app.state::<AppState>();
            let ptt_enabled = {
                let s = state
                    .settings
                    .lock()
                    .map_err(|_| "settings mutex poisoned".to_string())?;
                let enabled = s.ptt_enabled;
                drop(s);
                enabled
            };
            
            {
                let mut s = state
                    .settings
                    .lock()
                    .map_err(|_| "settings mutex poisoned".to_string())?;
                s.hotkey_accelerator = normalized.clone();
                s.hotkey_kind = "combo".to_string();
                s.hotkey_code = Some(code_str.clone());
                s.hotkey_mods = Some(mods_val.clone());
                if let Err(err) = state.settings_store.save(&s) {
                    eprintln!("[kiklet][hotkey] error: failed to save settings: {err}");
                    return Err(err.to_string());
                }
            }
            
            // PTT is managed by refreshSettings() only, not here
            // Just ensure it's stopped if disabled
            if !ptt_enabled {
                eprintln!("[kiklet][hotkey] PTT disabled, stopping tracker");
                let _ = hotkey_ptt::stop(&app);
            }
            
            Ok(normalized)
        }
        _ => Err(format!("unknown hotkey kind: {}", kind)),
    }
}

#[tauri::command]
pub fn reset_hotkey(app: tauri::AppHandle) -> Result<String, String> {
    let def = default_hotkey_str().to_string();
    eprintln!("[kiklet][hotkey] register: {def}");
    let chosen = match hotkey::register(&app, &def) {
        Ok(()) => def,
        Err(err) => {
            eprintln!("[kiklet][hotkey] error: {err}");
            let fb = fallback_hotkey_str().to_string();
            eprintln!("[kiklet][hotkey] fallback register: {fb}");
            hotkey::register(&app, &fb).map_err(|e| e.to_string())?;
            fb
        }
    };

    let state = app.state::<AppState>();
    {
        let mut s = state
            .settings
            .lock()
            .map_err(|_| "settings mutex poisoned".to_string())?;
        s.hotkey_accelerator = chosen.clone();
        if let Err(err) = state.settings_store.save(&s) {
            eprintln!("[kiklet][hotkey] error: failed to save settings: {err}");
            return Err(err.to_string());
        }
    }

    Ok(chosen)
}

#[tauri::command]
pub async fn transcribe_file(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let api_key = {
        let s = state
            .settings
            .lock()
            .map_err(|_| "settings mutex poisoned".to_string())?;
        s.openai_api_key.clone()
    };

    // Safety: avoid arbitrary file reads; require path to be inside recordings dir.
    let recordings_dir = recordings_dir(&app)
        .map_err(|e| format!("failed to get recordings_dir: {e}"))?;
    let requested = std::path::PathBuf::from(path);
    let requested = requested
        .canonicalize()
        .map_err(|_| "invalid path".to_string())?;
    let recordings_dir = recordings_dir
        .canonicalize()
        .map_err(|_| "recordings directory unavailable".to_string())?;
    if !requested.starts_with(&recordings_dir) {
        return Err("path must be inside the recordings folder".to_string());
    }

    openai::transcribe_whisper(&api_key, &requested)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_recording(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    // Safety: avoid arbitrary file deletes; require path to be inside recordings dir.
    let recordings_dir = recordings_dir(&app)
        .map_err(|e| format!("failed to get recordings_dir: {e}"))?;
    let requested = std::path::PathBuf::from(path);
    let requested = requested
        .canonicalize()
        .map_err(|_| "invalid path".to_string())?;
    let recordings_dir = recordings_dir
        .canonicalize()
        .map_err(|_| "recordings directory unavailable".to_string())?;
    if !requested.starts_with(&recordings_dir) {
        return Err("path must be inside the recordings folder".to_string());
    }

    std::fs::remove_file(&requested).map_err(|e| format!("failed to delete file: {e}"))?;

    let filename = requested
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "invalid filename".to_string())?
        .to_string();

    {
        let mut recs = state
            .recordings
            .lock()
            .map_err(|_| "recordings mutex poisoned".to_string())?;
        recs.retain(|r| r.filename != filename);
        state
            .storage
            .save_index(&recs)
            .map_err(|e| format!("failed to save index: {e}"))?;
    }

    let _ = app.emit("recordings_updated", ());
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingItem {
    pub id: String,
    pub filename: String,
    pub created_at: String,
    pub duration_sec: f64,
    pub size_bytes: u64,
    pub path: String,
}

fn to_item(storage: &crate::storage::Storage, e: &RecordingEntry) -> RecordingItem {
    let path = storage.recording_path(&e.filename);
    RecordingItem {
        id: e.id.clone(),
        filename: e.filename.clone(),
        created_at: e.created_at.clone(),
        duration_sec: e.duration_sec,
        size_bytes: e.size_bytes,
        path: path.to_string_lossy().to_string(),
    }
}

#[tauri::command]
pub fn start_recording(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    {
        let mut guard = state
            .active_recording
            .lock()
            .map_err(|_| "recording mutex poisoned".to_string())?;
        if guard.is_some() {
            return Err("already recording".to_string());
        }

        // Use SINGLE SOURCE OF TRUTH for recordings_dir
        let recordings_dir_path = recordings_dir(&app)
            .map_err(|e| format!("failed to get recordings_dir: {e}"))?;
        let active = audio::start_recording(&recordings_dir_path)
            .map_err(|e| format!("failed to start recording: {e}"))?;
        *guard = Some(active);
    }

    let _ = set_tray_recording_state(&app, true);
    let _ = emit_recording_state(&app, true);
    Ok(())
}

#[tauri::command]
pub fn stop_recording(app: AppHandle, state: State<'_, AppState>) -> Result<RecordingItem, String> {
    let active = {
        let mut guard = state
            .active_recording
            .lock()
            .map_err(|_| "recording mutex poisoned".to_string())?;
        guard
            .take()
            .ok_or_else(|| "not recording".to_string())?
    };

    let finished = audio::stop_recording(active).map_err(|e| format!("failed to stop: {e}"))?;

    let entry = RecordingEntry {
        id: finished.filename.trim_end_matches(".wav").to_string(),
        filename: finished.filename,
        created_at: finished.created_at,
        duration_sec: finished.duration_sec,
        size_bytes: finished.size_bytes,
    };

    {
        let mut recs = state
            .recordings
            .lock()
            .map_err(|_| "recordings mutex poisoned".to_string())?;
        recs.push(entry.clone());
        recs.sort_by(|a, b| b.filename.cmp(&a.filename));
        state
            .storage
            .save_index(&recs)
            .map_err(|e| format!("failed to save index: {e}"))?;
    }

    let _ = set_tray_recording_state(&app, false);
    let _ = emit_recording_state(&app, false);

    // Let the UI refresh without polling.
    let _ = app.emit("recordings_updated", ());

    Ok(to_item(&state.storage, &entry))
}

#[tauri::command]
pub fn list_recordings(state: State<'_, AppState>) -> Result<Vec<RecordingItem>, String> {
    let recs = state
        .recordings
        .lock()
        .map_err(|_| "recordings mutex poisoned".to_string())?;
    Ok(recs.iter().map(|e| to_item(&state.storage, e)).collect())
}

#[tauri::command]
pub fn open_recordings_folder(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let recordings_dir_path = recordings_dir(&app)
        .map_err(|e| format!("failed to get recordings_dir: {e}"))?;
    open_path_in_file_manager(&recordings_dir_path)
        .map_err(|e| format!("failed to open recordings folder: {e}"))?;
    // If the window is hidden, opening the folder should still be cheap; do not focus the app.
    let _app = app; // keep signature stable; may be useful later
    Ok(())
}

#[tauri::command]
pub fn reveal_in_finder(path: String) -> Result<(), String> {
    let p = PathBuf::from(path);
    reveal_path_in_file_manager(&p).map_err(|e| format!("failed to reveal: {e}"))?;
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgeResult {
    pub deleted_count: usize,
    pub kept_count: usize,
}

#[tauri::command]
pub fn purge_old_recordings(
    app: AppHandle,
    state: State<'_, AppState>,
    days: u32,
) -> Result<PurgeResult, String> {
    eprintln!("[kiklet][commands] purge_old_recordings invoked days={}", days);
    let (deleted_count, kept_count) = state
        .storage
        .purge_old_recordings(&app, days)
        .map_err(|e| {
            eprintln!("[kiklet][commands] purge_old_recordings error: {}", e);
            format!("purge failed: {e}")
        })?;

    eprintln!("[kiklet][commands] purge_old_recordings deleted={}, kept={}", deleted_count, kept_count);

    // Reload recordings in state
    {
        let mut recs = state
            .recordings
            .lock()
            .map_err(|_| "recordings mutex poisoned".to_string())?;
        *recs = state
            .storage
            .load_or_rebuild_index()
            .map_err(|e| format!("failed to reload index: {e}"))?;
        eprintln!("[kiklet][commands] purge_old_recordings: state reloaded, recordings count={}", recs.len());
    }

    // Emit event to refresh UI
    let _ = app.emit("recordings_updated", ());
    eprintln!("[kiklet][commands] purge_old_recordings: emitted recordings_updated event");

    Ok(PurgeResult {
        deleted_count,
        kept_count,
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearResult {
    pub deleted_count: usize,
    pub recordings_dir: String,
    pub recordings_json: String,
    pub files_on_disk_before: usize,
    pub files_on_disk_after: usize,
    pub index_count_before: usize,
    pub index_count_after: usize,
    pub failed_deletes: Vec<crate::storage::FailedDelete>,
}

#[tauri::command]
pub fn clear_all_recordings(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ClearResult, String> {
    println!("[kiklet][commands] clear_all_recordings INVOKED");
    eprintln!("[kiklet][commands] clear_all_recordings INVOKED");

    let recordings_dir_path = recordings_dir(&app).map_err(|e| format!("failed to get recordings_dir: {e}"))?;
    let recordings_json_path = state.storage.index_path.clone();

    println!(
        "[kiklet][commands] recordings_dir={}",
        recordings_dir_path.to_string_lossy()
    );
    println!(
        "[kiklet][commands] recordings_json={}",
        recordings_json_path.to_string_lossy()
    );

    let files_on_disk_before = count_wav_files(&recordings_dir_path);
    println!(
        "[kiklet][commands] wav_on_disk_before={}",
        files_on_disk_before
    );

    let report: ClearAllReport = state.storage.clear_all_recordings(&app).map_err(|e| {
        eprintln!("[kiklet][commands] clear_all_recordings error: {}", e);
        format!("clear failed: {e}")
    })?;

    let files_on_disk_after = count_wav_files(&recordings_dir_path);
    println!(
        "[kiklet][commands] wav_on_disk_after={}",
        files_on_disk_after
    );

    // Re-read index count from disk after save (proof)
    let index_count_after = match std::fs::read_to_string(&recordings_json_path) {
        Ok(raw) => match serde_json::from_str::<RecordingsIndex>(&raw) {
            Ok(idx) => idx.recordings.len(),
            Err(_) => 0,
        },
        Err(_) => 0,
    };

    eprintln!(
        "[kiklet][commands] clear_all_recordings done deleted={} failed={}",
        report.deleted_count,
        report.failed_deletes.len()
    );
    println!(
        "[kiklet][commands] clear_all_recordings done deleted={} failed={}",
        report.deleted_count,
        report.failed_deletes.len()
    );
    println!(
        "[kiklet][commands] clear_all_recordings summary index_before={} index_after={} wav_before={} wav_after={}",
        report.index_count_before,
        index_count_after,
        files_on_disk_before,
        files_on_disk_after
    );

    // Clear recordings in state
    {
        let mut recs = state
            .recordings
            .lock()
            .map_err(|_| "recordings mutex poisoned".to_string())?;
        *recs = Vec::new();
        eprintln!("[kiklet][commands] clear_all_recordings: state cleared, recordings count=0");
    }

    // Emit event to refresh UI
    let _ = app.emit("recordings_updated", ());
    eprintln!("[kiklet][commands] clear_all_recordings: emitted recordings_updated event");

    Ok(ClearResult {
        deleted_count: report.deleted_count,
        recordings_dir: recordings_dir_path.to_string_lossy().to_string(),
        recordings_json: recordings_json_path.to_string_lossy().to_string(),
        files_on_disk_before: report.files_on_disk_before,
        files_on_disk_after,
        index_count_before: report.index_count_before,
        index_count_after,
        failed_deletes: report.failed_deletes,
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugStorageDump {
    pub recordings_dir: String,
    pub recordings_json: String,
    pub first3: Vec<serde_json::Value>,
}

/// PROOF command: returns absolute paths and first entries from recordings.json on disk.
#[tauri::command]
pub fn debug_dump_storage_paths(app: AppHandle, state: State<'_, AppState>) -> Result<DebugStorageDump, String> {
    let recordings_dir_path = recordings_dir(&app)
        .map_err(|e| format!("failed to get recordings_dir: {e}"))?;
    let index_path = state.storage.index_path.clone();

    let mut first3: Vec<serde_json::Value> = Vec::new();

    if index_path.exists() {
        match std::fs::read_to_string(&index_path) {
            Ok(raw) => match serde_json::from_str::<RecordingsIndex>(&raw) {
                Ok(idx) => {
                    for rec in idx.recordings.iter().take(3) {
                        first3.push(serde_json::json!({
                            "id": rec.id,
                            "filename": rec.filename,
                            "created_at": rec.created_at,
                        }));
                    }
                }
                Err(e) => {
                    first3.push(serde_json::json!({
                        "error": "failed_to_parse_recordings_json",
                        "detail": e.to_string(),
                    }));
                }
            },
            Err(e) => {
                first3.push(serde_json::json!({
                    "error": "failed_to_read_recordings_json",
                    "detail": e.to_string(),
                }));
            }
        }
    } else {
        first3.push(serde_json::json!({
            "note": "recordings.json does not exist on disk",
        }));
    }

    Ok(DebugStorageDump {
        recordings_dir: recordings_dir_path.to_string_lossy().to_string(),
        recordings_json: index_path.to_string_lossy().to_string(),
        first3,
    })
}

#[tauri::command]
pub fn debug_ping() -> String {
    println!("[kiklet][commands] debug_ping");
    "pong".to_string()
}

#[cfg(debug_assertions)]
#[tauri::command]
pub fn debug_print_recordings_paths(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    use std::time::SystemTime;
    use chrono::DateTime;
    
    eprintln!("[kiklet][debug] debug_print_recordings_paths invoked");
    
    // Use SINGLE SOURCE OF TRUTH
    let recordings_dir_path = recordings_dir(&app)
        .map_err(|e| format!("failed to get recordings_dir: {e}"))?;
    eprintln!("[kiklet][debug] recordings_dir={:?}", recordings_dir_path);
    
    let mut files = Vec::new();
    let mut file_count = 0;
    
    if let Ok(entries) = std::fs::read_dir(&recordings_dir_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ext == "wav" {
                    file_count += 1;
                    let filename = path.file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    
                    let mut file_info = serde_json::json!({
                        "filename": filename,
                    });
                    
                    if let Ok(meta) = std::fs::metadata(&path) {
                        if let Ok(mtime) = meta.modified() {
                            if let Ok(dur) = mtime.duration_since(SystemTime::UNIX_EPOCH) {
                                file_info["mtime"] = serde_json::json!(dur.as_secs());
                                if let Some(dt) = chrono::DateTime::from_timestamp(dur.as_secs() as i64, 0) {
                                    file_info["mtime_iso"] = serde_json::json!(dt.to_rfc3339());
                                }
                            }
                        }
                        file_info["size"] = serde_json::json!(meta.len());
                    }
                    
                    if files.len() < 10 {
                        files.push(file_info);
                    }
                }
            }
        }
    }
    
    eprintln!("[kiklet][debug] found {} .wav files on disk", file_count);
    
    Ok(serde_json::json!({
        "recordings_dir": recordings_dir_path.to_string_lossy(),
        "file_count": file_count,
        "first_files": files,
    }))
}

use std::path::{Path, PathBuf};

fn open_path_in_file_manager(path: &Path) -> Result<(), std::io::Error> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).status()?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer").arg(path).status()?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(path).status()?;
        return Ok(());
    }
}

fn reveal_path_in_file_manager(path: &Path) -> Result<(), std::io::Error> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg("-R").arg(path).status()?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(path)
            .status()?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Best effort: open containing folder.
        if let Some(parent) = path.parent() {
            std::process::Command::new("xdg-open").arg(parent).status()?;
        }
        return Ok(());
    }
}

fn count_wav_files(dir: &std::path::Path) -> usize {
    let mut n = 0usize;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) == Some("wav") {
                n += 1;
            }
        }
    }
    n
}

#[tauri::command]
pub fn ptt_start(
    app: tauri::AppHandle,
    code: Option<String>,
    mods: Option<crate::settings::HotkeyMods>,
    accelerator: String,
) -> Result<bool, String> {
    eprintln!("[kiklet][ptt] ptt_start command code={:?} accelerator={}", code, accelerator);
    let config = hotkey_ptt::PttConfig {
        accelerator: accelerator.clone(),
        code: code.clone(),
        mods: mods.clone(),
    };
    match hotkey_ptt::start(&app, config) {
        Ok(()) => {
            eprintln!("[kiklet][ptt] start ok");
            Ok(true)
        }
        Err(e) => {
            eprintln!("[kiklet][ptt] start failed err={}", e);
            if e.contains("Accessibility") || e.contains("accessibility") || e.contains("permission") {
                Err(format!("need_accessibility: {}", e))
            } else {
                Err(e)
            }
        }
    }
}

#[tauri::command]
pub fn ptt_stop(app: tauri::AppHandle) -> Result<bool, String> {
    hotkey_ptt::stop(&app)?;
    Ok(true)
}

#[tauri::command]
pub fn ptt_status(app: tauri::AppHandle) -> Result<bool, String> {
    Ok(hotkey_ptt::status(&app))
}

#[tauri::command]
pub fn set_ptt_threshold_ms(
    app: tauri::AppHandle,
    threshold_ms: u64,
) -> Result<SettingsDto, String> {
    let state = app.state::<AppState>();
    let mut s = state
        .settings
        .lock()
        .map_err(|_| "settings mutex poisoned".to_string())?;
    
    s.ptt_threshold_ms = threshold_ms;
    s.ptt_enabled = threshold_ms > 0;
    
    state
        .settings_store
        .save(&s)
        .map_err(|e| format!("failed to save settings: {}", e))?;
    
    Ok(SettingsDto {
        has_openai_api_key: !s.openai_api_key.trim().is_empty(),
        openai_api_key: s.openai_api_key.clone(),
        autoinsert_enabled: s.autoinsert_enabled,
        hotkey_accelerator: s.hotkey_accelerator.clone(),
        hotkey_code: s.hotkey_code.clone(),
        hotkey_mods: s.hotkey_mods.clone(),
        ptt_enabled: s.ptt_enabled,
        ptt_threshold_ms: s.ptt_threshold_ms,
        translate_target: s.translate_target.clone(),
        translate_model: s.translate_model.clone(),
    })
}

#[tauri::command]
pub async fn list_models(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    
    // Simple in-memory cache (5 minutes)
    static CACHE: std::sync::OnceLock<Arc<Mutex<Option<(Vec<String>, Instant)>>>> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| Arc::new(Mutex::new(None)));
    
    // Check cache
    {
        let guard = cache.lock().map_err(|_| "cache mutex poisoned".to_string())?;
        if let Some((models, timestamp)) = guard.as_ref() {
            if timestamp.elapsed() < Duration::from_secs(300) {
                eprintln!("[kiklet][translate] list_models cache hit");
                return Ok(models.clone());
            }
        }
    }
    
    // Get API key
    let api_key = {
        let s = state
            .settings
            .lock()
            .map_err(|_| "settings mutex poisoned".to_string())?;
        s.openai_api_key.trim().to_string()
    };
    
    if api_key.is_empty() {
        eprintln!("[kiklet][translate] list_models no_api_key");
        return Ok(vec![]);
    }
    
    eprintln!("[kiklet][translate] list_models fetching...");
    
    let client = reqwest::Client::builder()
        .user_agent("kiklet/0.1")
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("failed to create HTTP client: {}", e))?;
    
    let resp = client
        .get("https://api.openai.com/v1/models")
        .bearer_auth(&api_key)
        .send()
        .await
        .map_err(|e| format!("HTTP error: {}", e))?;
    
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("OpenAI API error {}: {}", status, body));
    }
    
    #[derive(serde::Deserialize)]
    struct Model {
        id: String,
    }
    
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<Model>,
    }
    
    let json: ModelsResponse = resp.json().await.map_err(|e| format!("JSON parse error: {}", e))?;
    
    let raw_count = json.data.len();
    
    // Filter: only LLM models (chat/completions)
    // Allow: gpt-*, o* (o1, o3, o4), text-*
    // Deny: image, dall, sora, whisper, tts, embedding, moderation, realtime, audio (case-insensitive)
    let models: Vec<String> = json
        .data
        .into_iter()
        .map(|m| m.id)
        .filter(|id| {
            let id_lower = id.to_lowercase();
            
            // Deny patterns (case-insensitive)
            if id_lower.contains("image")
                || id_lower.contains("dall")
                || id_lower.contains("sora")
                || id_lower.contains("whisper")
                || id_lower.contains("tts")
                || id_lower.contains("embedding")
                || id_lower.contains("moderation")
                || id_lower.contains("realtime")
                || id_lower.contains("audio")
            {
                return false;
            }
            
            // Allow patterns
            id.starts_with("gpt-")
                || (id.starts_with("o") && id.len() > 1 && id.chars().nth(1).map_or(false, |c| c.is_ascii_digit()))
                || id.starts_with("text-")
        })
        .collect();
    
    eprintln!("[kiklet][models] raw={} filtered={}", raw_count, models.len());
    
    // Sort: recommended first (gpt-4.1 variants, then gpt-4o, then alphabetically)
    let mut sorted = models;
    sorted.sort_by(|a, b| {
        let a_rec = if a.starts_with("gpt-4.1") {
            0
        } else if a.starts_with("gpt-4o") {
            1
        } else {
            2
        };
        let b_rec = if b.starts_with("gpt-4.1") {
            0
        } else if b.starts_with("gpt-4o") {
            1
        } else {
            2
        };
        a_rec.cmp(&b_rec).then_with(|| a.cmp(b))
    });
    
    eprintln!("[kiklet][translate] list_models ok count={}", sorted.len());
    
    // Update cache
    {
        let mut guard = cache.lock().map_err(|_| "cache mutex poisoned".to_string())?;
        *guard = Some((sorted.clone(), Instant::now()));
    }
    
    Ok(sorted)
}

#[tauri::command]
pub async fn translate_text(
    state: State<'_, AppState>,
    text: String,
    target_language: String,
    model: String,
) -> Result<String, String> {
    use std::time::Duration;
    
    eprintln!(
        "[kiklet][translate] start model={} lang={} len={}",
        model,
        target_language,
        text.len()
    );
    
    // Get API key
    let api_key = {
        let s = state
            .settings
            .lock()
            .map_err(|_| "settings mutex poisoned".to_string())?;
        s.openai_api_key.trim().to_string()
    };
    
    if api_key.is_empty() {
        return Err("missing OpenAI API key".to_string());
    }
    
    let prompt = format!(
        "Translate the following text to {}. Return only the translated text.\n\nTEXT:\n{}",
        target_language, text
    );
    
    let client = reqwest::Client::builder()
        .user_agent("kiklet/0.1")
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("failed to create HTTP client: {}", e))?;
    
    #[derive(serde::Serialize)]
    struct ChatRequest {
        model: String,
        messages: Vec<ChatMessage>,
        temperature: f64,
    }
    
    #[derive(serde::Serialize)]
    struct ChatMessage {
        role: String,
        content: String,
    }
    
    let req = ChatRequest {
        model: model.clone(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
        }],
        temperature: 0.3,
    };
    
    let resp = client
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(&api_key)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("HTTP error: {}", e))?;
    
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        eprintln!("[kiklet][translate] fail err={}: {}", status, body);
        return Err(format!("OpenAI API error {}: {}", status, body));
    }
    
    #[derive(serde::Deserialize)]
    struct ChatChoice {
        message: ChatResponseMessage,
    }
    
    #[derive(serde::Deserialize)]
    struct ChatResponseMessage {
        content: String,
    }
    
    #[derive(serde::Deserialize)]
    struct ChatResponse {
        choices: Vec<ChatChoice>,
    }
    
    let json: ChatResponse = resp.json().await.map_err(|e| format!("JSON parse error: {}", e))?;
    
    let translated = json
        .choices
        .first()
        .and_then(|c| Some(c.message.content.trim().to_string()))
        .ok_or_else(|| "no response from API".to_string())?;
    
    eprintln!(
        "[kiklet][translate] ok chars_in={} chars_out={}",
        text.len(),
        translated.len()
    );
    
    Ok(translated)
}

#[tauri::command]
pub fn set_translate_target(
    state: State<'_, AppState>,
    target: Option<String>,
) -> Result<SettingsDto, String> {
    let mut s = state
        .settings
        .lock()
        .map_err(|_| "settings mutex poisoned".to_string())?;
    
    s.translate_target = target;
    
    state
        .settings_store
        .save(&s)
        .map_err(|e| format!("failed to save settings: {}", e))?;
    
    Ok(SettingsDto {
        has_openai_api_key: !s.openai_api_key.trim().is_empty(),
        openai_api_key: s.openai_api_key.clone(),
        autoinsert_enabled: s.autoinsert_enabled,
        hotkey_accelerator: s.hotkey_accelerator.clone(),
        hotkey_code: s.hotkey_code.clone(),
        hotkey_mods: s.hotkey_mods.clone(),
        ptt_enabled: s.ptt_enabled,
        ptt_threshold_ms: s.ptt_threshold_ms,
        translate_target: s.translate_target.clone(),
        translate_model: s.translate_model.clone(),
    })
}

#[tauri::command]
pub fn set_translate_model(
    state: State<'_, AppState>,
    model: String,
) -> Result<SettingsDto, String> {
    let mut s = state
        .settings
        .lock()
        .map_err(|_| "settings mutex poisoned".to_string())?;
    
    s.translate_model = model;
    
    state
        .settings_store
        .save(&s)
        .map_err(|e| format!("failed to save settings: {}", e))?;
    
    Ok(SettingsDto {
        has_openai_api_key: !s.openai_api_key.trim().is_empty(),
        openai_api_key: s.openai_api_key.clone(),
        autoinsert_enabled: s.autoinsert_enabled,
        hotkey_accelerator: s.hotkey_accelerator.clone(),
        hotkey_code: s.hotkey_code.clone(),
        hotkey_mods: s.hotkey_mods.clone(),
        ptt_enabled: s.ptt_enabled,
        ptt_threshold_ms: s.ptt_threshold_ms,
        translate_target: s.translate_target.clone(),
        translate_model: s.translate_model.clone(),
    })
}


