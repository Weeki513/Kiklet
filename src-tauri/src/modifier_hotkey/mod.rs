#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use tauri::AppHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierKey {
    Cmd,
    Option,
    Shift,
    Ctrl,
    Win,
}

pub fn start(app: &AppHandle, modifier: ModifierKey) -> Result<(), String> {
    eprintln!("[kiklet][mhotkey] start modifier={:?}", modifier);
    #[cfg(target_os = "macos")]
    return macos::start(app, modifier);
    #[cfg(target_os = "windows")]
    return windows::start(app, modifier);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Err("modifier-only hotkeys not supported on this platform".to_string())
}

pub fn stop(app: &AppHandle) -> Result<(), String> {
    eprintln!("[kiklet][mhotkey] stop");
    #[cfg(target_os = "macos")]
    return macos::stop(app);
    #[cfg(target_os = "windows")]
    return windows::stop(app);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Ok(())
}

pub fn status(app: &AppHandle) -> bool {
    #[cfg(target_os = "macos")]
    return macos::status(app);
    #[cfg(target_os = "windows")]
    return windows::status(app);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    false
}

