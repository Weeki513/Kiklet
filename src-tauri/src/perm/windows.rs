use super::{PermissionCheckResult, RequestAccessibilityResult};

pub fn check_permissions() -> Result<PermissionCheckResult, String> {
    Ok(PermissionCheckResult {
        ok: true,
        need_accessibility: false,
    })
}

pub fn request_accessibility() -> Result<RequestAccessibilityResult, String> {
    Ok(RequestAccessibilityResult { requested: false })
}

use super::{PermissionCheckResult, PermissionStatus};

pub fn check_permissions() -> Result<PermissionCheckResult, String> {
    Ok(PermissionCheckResult {
        ok: true,
        status: PermissionStatus::Ok,
        detail: Some("windows_no_tcc".to_string()),
        stderr: None,
    })
}


