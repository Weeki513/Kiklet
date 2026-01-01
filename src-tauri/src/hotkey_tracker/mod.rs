#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use tauri::AppHandle;

use crate::modifier_hotkey;

#[derive(Debug, Clone)]
pub struct HotkeyConfig {
    pub kind: String, // "modifier" | "combo"
    pub code: Option<String>, // For combo: e.g. "KeyS", "Digit1", "ArrowLeft"
    pub mods: Option<crate::settings::HotkeyMods>, // For combo
    pub modifier: Option<modifier_hotkey::ModifierKey>, // For modifier-only
}

pub fn start(app: &AppHandle, config: HotkeyConfig) -> Result<(), String> {
    eprintln!("[kiklet][hotkey_tracker] start kind={}", config.kind);
    #[cfg(target_os = "macos")]
    return macos::start(app, config);
    #[cfg(target_os = "windows")]
    return windows::start(app, config);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Err("hotkey tracker not supported on this platform".to_string())
}

pub fn stop(app: &AppHandle) -> Result<(), String> {
    eprintln!("[kiklet][hotkey_tracker] stop");
    #[cfg(target_os = "macos")]
    return macos::stop(app);
    #[cfg(target_os = "windows")]
    return windows::stop(app);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Ok(())
}

