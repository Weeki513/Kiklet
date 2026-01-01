use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionCheckResult {
    pub ok: bool,
    pub need_accessibility: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestAccessibilityResult {
    pub requested: bool,
}

pub fn check_permissions() -> Result<PermissionCheckResult, String> {
    #[cfg(target_os = "macos")]
    return macos::check_permissions();
    #[cfg(target_os = "windows")]
    return windows::check_permissions();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Ok(PermissionCheckResult {
        ok: true,
        need_accessibility: false,
    })
}

pub fn request_accessibility() -> Result<RequestAccessibilityResult, String> {
    #[cfg(target_os = "macos")]
    return macos::request_accessibility();
    #[cfg(target_os = "windows")]
    return windows::request_accessibility();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Ok(RequestAccessibilityResult { requested: false })
}

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;


