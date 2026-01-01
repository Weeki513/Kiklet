use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};

use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[derive(Debug, Default)]
pub struct HotkeyState {
    pub current: Mutex<Option<String>>,
    pub last_error: Mutex<Option<String>>,
}

pub fn current(app: &AppHandle) -> Option<String> {
    app.try_state::<HotkeyState>()
        .and_then(|s| s.current.lock().ok().and_then(|g| g.clone()))
}

pub fn unregister(app: &AppHandle) -> Result<(), String> {
    let Some(state) = app.try_state::<HotkeyState>() else {
        return Ok(());
    };
    let prev = state
        .current
        .lock()
        .map_err(|_| "hotkey mutex poisoned".to_string())?
        .take();
    if let Some(prev) = prev {
        eprintln!("[kiklet][hotkey] unregister: {prev}");
        app.global_shortcut()
            .unregister(prev.as_str())
            .map_err(|e| e.to_string())?;
        eprintln!("[kiklet][hotkey] unregister ok");
    }
    Ok(())
}

pub fn register(app: &AppHandle, accelerator: &str) -> Result<(), String> {
    eprintln!("[kiklet][hotkey] register: {accelerator}");
    let state = app
        .try_state::<HotkeyState>()
        .ok_or_else(|| "hotkey state missing".to_string())?;

    // Capture previous so we can rollback if new registration fails.
    let prev = state
        .current
        .lock()
        .map_err(|_| "hotkey mutex poisoned".to_string())?
        .clone();

    // Unregister old first (per spec).
    unregister(app)?;

    let acc = accelerator.to_string();
    let reg_res = app.global_shortcut().on_shortcut(
        acc.as_str(),
        move |app_handle, _shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            eprintln!("[kiklet][hotkey] triggered");
            let _ = app_handle.emit("hotkey:toggle-record", ());
        },
    );
    if let Err(err) = reg_res {
        // Rollback: best-effort re-register previous hotkey.
        if let Some(prev) = prev.clone() {
            eprintln!("[kiklet][hotkey] rollback register: {prev}");
            let rollback_res = app.global_shortcut().on_shortcut(
                prev.as_str(),
                move |app_handle, _shortcut, event| {
                    if event.state != ShortcutState::Pressed {
                        return;
                    }
                    eprintln!("[kiklet][hotkey] triggered");
                    let _ = app_handle.emit("hotkey:toggle-record", ());
                },
            );
            match rollback_res {
                Ok(()) => {
                    eprintln!("[kiklet][hotkey] rollback ok");
                    if let Ok(mut g) = state.current.lock() {
                        *g = Some(prev);
                    }
                }
                Err(err2) => {
                    eprintln!("[kiklet][hotkey] rollback failed: {err2}");
                    if let Ok(mut g) = state.current.lock() {
                        *g = None;
                    }
                    return Err(format!(
                        "failed to register hotkey: {}; rollback to previous failed: {}",
                        err,
                        err2
                    ));
                }
            }
            return Err(err.to_string());
        }
        if let Ok(mut g) = state.current.lock() {
            *g = None;
        }
        return Err(err.to_string());
    }

    *state
        .current
        .lock()
        .map_err(|_| "hotkey mutex poisoned".to_string())? = Some(acc.clone());

    *state
        .last_error
        .lock()
        .map_err(|_| "hotkey mutex poisoned".to_string())? = None;

    Ok(())
}

pub fn set_error(app: &AppHandle, err: String) {
    if let Some(state) = app.try_state::<HotkeyState>() {
        if let Ok(mut g) = state.last_error.lock() {
            *g = Some(err);
        }
    }
}

pub fn is_enabled(app: &AppHandle) -> bool {
    current(app).is_some()
}


