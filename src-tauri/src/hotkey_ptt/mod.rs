#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use tauri::AppHandle;

#[derive(Debug, Clone)]
pub struct PttConfig {
    pub accelerator: String, // e.g. "Cmd+Shift+Space", "Shift+1", "Insert"
    pub code: Option<String>, // Physical code: "KeyS", "Digit1", "Insert"
    pub mods: Option<crate::settings::HotkeyMods>, // Modifiers for combo
}

pub fn start(app: &AppHandle, config: PttConfig) -> Result<(), String> {
    eprintln!("[kiklet][ptt] start accelerator={}", config.accelerator);
    #[cfg(target_os = "macos")]
    return macos::start(app, config);
    #[cfg(target_os = "windows")]
    return windows::start(app, config);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Err("PTT not supported on this platform".to_string())
}

pub fn stop(app: &AppHandle) -> Result<(), String> {
    eprintln!("[kiklet][ptt] stop");
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

