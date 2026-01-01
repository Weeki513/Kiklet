use tauri::{AppHandle, Emitter, State};

use crate::audio;
use crate::storage::RecordingEntry;
use crate::{emit_recording_state, notify, set_tray_recording_state, AppState};

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

    let _ = notify(&app, "Recording started");
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

    let _ = notify(&app, "Recording stopped");
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


