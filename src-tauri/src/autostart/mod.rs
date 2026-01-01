use tauri::AppHandle;

pub fn enable(app: &AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return macos::enable(app);
    #[cfg(target_os = "windows")]
    return windows::enable(app);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = app;
        Err("autostart is not supported on this platform yet".to_string())
    }
}

pub fn disable(app: &AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return macos::disable(app);
    #[cfg(target_os = "windows")]
    return windows::disable(app);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = app;
        Err("autostart is not supported on this platform yet".to_string())
    }
}

pub fn is_enabled(app: &AppHandle) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    return macos::is_enabled(app);
    #[cfg(target_os = "windows")]
    return windows::is_enabled(app);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = app;
        Ok(false)
    }
}

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;


