use tauri::{AppHandle, Emitter, State};

use crate::audio;
use crate::openai;
use crate::storage::RecordingEntry;
use crate::{emit_recording_state, set_tray_recording_state, AppState};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub openai_api_key: String,
    pub has_openai_api_key: bool,
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
    })
}

#[tauri::command]
pub fn set_openai_api_key(state: State<'_, AppState>, api_key: String) -> Result<(), String> {
    let trimmed = api_key.trim().to_string();
    {
        let mut s = state
            .settings
            .lock()
            .map_err(|_| "settings mutex poisoned".to_string())?;
        s.openai_api_key = trimmed;
        state
            .settings_store
            .save(&s)
            .map_err(|e| {
                eprintln!("[kiklet][settings] save failed: {e}");
                format!("failed to save settings: {e}")
            })?;
        eprintln!(
            "[kiklet][settings] saved to {}",
            state.settings_store.path.display()
        );
    }
    Ok(())
}

#[tauri::command]
pub fn debug_settings_path(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.settings_store.path.to_string_lossy().to_string())
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


