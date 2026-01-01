use tauri::{AppHandle, Emitter, Manager, State};

use crate::audio;
use crate::autostart;
use crate::deliver;
use crate::hotkey;
use crate::openai;
use crate::perm;
use crate::storage::RecordingEntry;
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
            eprintln!(
                "[kiklet][deliver] ok platform={} len={} mode={:?} detail={}",
                platform,
                text.len(),
                res.mode,
                res.detail.clone().unwrap_or_else(|| "-".to_string())
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
    // allow single key like "Ins"
    if a.eq_ignore_ascii_case("ins") || a.eq_ignore_ascii_case("insert") {
        return Ok(());
    }
    Err("hotkey must contain a modifier (or be a single key like Ins)".to_string())
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

#[tauri::command]
pub fn set_hotkey(app: tauri::AppHandle, accelerator: String) -> Result<String, String> {
    let acc = accelerator.trim().to_string();
    validate_hotkey(&acc)?;

    let prev = hotkey::current(&app);
    eprintln!("[kiklet][hotkey] attempt register {acc}");
    if let Err(err) = hotkey::register(&app, &acc) {
        if let Some(prev) = prev {
            // If register failed, hotkey::register already attempted rollback; log what we expect.
            eprintln!("[kiklet][hotkey] failed: {err}");
            eprintln!("[kiklet][hotkey] rollback to {prev} attempted");
        } else {
            eprintln!("[kiklet][hotkey] failed: {err}");
        }
        return Err(err);
    }

    let state = app.state::<AppState>();
    {
        let mut s = state
            .settings
            .lock()
            .map_err(|_| "settings mutex poisoned".to_string())?;
        s.hotkey_accelerator = acc.clone();
        if let Err(err) = state.settings_store.save(&s) {
            eprintln!("[kiklet][hotkey] error: failed to save settings: {err}");
            return Err(err.to_string());
        }
    }

    Ok(acc)
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
pub async fn transcribe_file(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let api_key = {
        let s = state
            .settings
            .lock()
            .map_err(|_| "settings mutex poisoned".to_string())?;
        s.openai_api_key.clone()
    };

    // Safety: avoid arbitrary file reads; require path to be inside recordings dir.
    let recordings_dir = state.storage.recordings_dir.clone();
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
pub fn delete_recording(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<(), String> {
    // Safety: avoid arbitrary file deletes; require path to be inside recordings dir.
    let recordings_dir = state.storage.recordings_dir.clone();
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

        let active = audio::start_recording(&state.storage.recordings_dir)
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
    open_path_in_file_manager(&state.storage.recordings_dir)
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


