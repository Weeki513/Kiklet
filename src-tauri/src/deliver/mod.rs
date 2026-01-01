use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryMode {
    Insert,
    Copy,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryResult {
    pub mode: DeliveryMode,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub fn deliver(text: &str, attempt_insert: bool) -> Result<DeliveryResult, String> {
    #[cfg(target_os = "macos")]
    return macos::deliver(text, attempt_insert);
    #[cfg(target_os = "windows")]
    return windows::deliver(text, attempt_insert);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = text;
        Ok(DeliveryResult {
            mode: DeliveryMode::Copy,
            ok: false,
            detail: Some("unsupported_platform".to_string()),
        })
    }
}

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;


