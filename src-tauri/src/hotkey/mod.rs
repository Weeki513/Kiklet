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

// Try to register a single accelerator (does NOT unregister old)
fn try_register_single(app: &AppHandle, accelerator: &str) -> Result<(), String> {
    let acc = accelerator.to_string();
    app.global_shortcut().on_shortcut(
        acc.as_str(),
        move |app_handle, _shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            eprintln!("[kiklet][hotkey] triggered");
            let _ = app_handle.emit("hotkey:toggle-record", ());
        },
    )
    .map_err(|e| e.to_string())
}

pub fn register(app: &AppHandle, accelerator: &str) -> Result<(), String> {
    eprintln!("[kiklet][hotkey] register: {accelerator}");
    let state = app
        .try_state::<HotkeyState>()
        .ok_or_else(|| "hotkey state missing".to_string())?;

    // Try to register new WITHOUT unregistering old first
    match try_register_single(app, accelerator) {
        Ok(()) => {
            // Success: now unregister old
            let _prev = state
                .current
                .lock()
                .map_err(|_| "hotkey mutex poisoned".to_string())?
                .clone();
            if _prev.is_some() {
                let _ = unregister(app); // Best-effort, ignore errors
            }
            
            // Update state
            *state
                .current
                .lock()
                .map_err(|_| "hotkey mutex poisoned".to_string())? = Some(accelerator.to_string());
            *state
                .last_error
                .lock()
                .map_err(|_| "hotkey mutex poisoned".to_string())? = None;
            
            eprintln!("[kiklet][hotkey] register_ok {}", accelerator);
            Ok(())
        }
        Err(err) => {
            eprintln!("[kiklet][hotkey] register_failed {}: {}", accelerator, err);
            Err(err)
        }
    }
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


