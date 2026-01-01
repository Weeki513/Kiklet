use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use tauri::Manager;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("tauri error: {0}")]
    Tauri(#[from] tauri::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct Settings {
    #[serde(default)]
    pub autostart_enabled: bool,
    #[serde(default)]
    pub autoinsert_enabled: bool,
    #[serde(default)]
    pub hotkey_accelerator: String,
    #[serde(default)]
    pub hotkey_kind: String, // "modifier" | "combo"
    #[serde(default)]
    pub hotkey_code: Option<String>, // For combo: e.g. "Digit1"
    #[serde(default)]
    pub hotkey_mods: Option<HotkeyMods>, // For combo
    #[serde(default = "default_ptt_enabled")]
    pub ptt_enabled: bool,
    #[serde(default = "default_ptt_threshold_ms")]
    pub ptt_threshold_ms: u64,
    pub openai_api_key: String,
    #[serde(default)]
    pub translate_target: Option<String>, // null = не переводить
    #[serde(default = "default_translate_model")]
    pub translate_model: String,
}

fn default_translate_model() -> String {
    "gpt-4o".to_string() // Fallback, будет заменено на доступную модель при загрузке
}

fn default_ptt_enabled() -> bool {
    false
}

fn default_ptt_threshold_ms() -> u64 {
    300
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HotkeyMods {
    #[serde(default)]
    pub cmd: bool,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
}

#[derive(Debug, Clone)]
pub struct SettingsStore {
    pub path: PathBuf,
}

impl SettingsStore {
    pub fn new(app: &tauri::AppHandle) -> Result<Self, SettingsError> {
        let dir = app.path().app_data_dir()?;
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            path: dir.join("settings.json"),
        })
    }

    pub fn load(&self) -> Result<Settings, SettingsError> {
        if !self.path.exists() {
            return Ok(Settings::default());
        }
        let f = File::open(&self.path)?;
        let r = BufReader::new(f);
        Ok(serde_json::from_reader(r)?)
    }

    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let tmp = self.path.with_extension("json.tmp");
        {
            let f = File::create(&tmp)?;
            let mut w = BufWriter::new(f);
            serde_json::to_writer_pretty(&mut w, settings)?;
            w.write_all(b"\n")?;
            w.flush()?;
        }

        set_private_permissions(&tmp);
        if self.path.exists() {
            let _ = std::fs::remove_file(&self.path);
        }
        std::fs::rename(&tmp, &self.path)?;
        set_private_permissions(&self.path);
        Ok(())
    }
}

fn set_private_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}


