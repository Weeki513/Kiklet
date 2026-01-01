use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use tauri::Manager;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("tauri path resolver error: {0}")]
    TauriPath(#[from] tauri::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("wav error: {0}")]
    Wav(#[from] hound::Error),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingEntry {
    pub id: String,
    pub filename: String,
    pub created_at: String,
    pub duration_sec: f64,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecordingsIndex {
    pub version: u32,
    pub recordings: Vec<RecordingEntry>,
}

#[derive(Debug, Clone)]
pub struct Storage {
    pub app_data_dir: PathBuf,
    pub recordings_dir: PathBuf,
    pub index_path: PathBuf,
}

fn index_filename() -> &'static str {
    "recordings.json"
}

fn recordings_dirname() -> &'static str {
    "recordings"
}

fn debug_log(msg: &str) {
    if cfg!(debug_assertions) {
        eprintln!("[kiklet][storage] {msg}");
    }
}

impl Storage {
    pub fn new(app: &tauri::AppHandle) -> Result<Self, StorageError> {
        let app_data_dir = app.path().app_data_dir()?;
        let recordings_dir = app_data_dir.join(recordings_dirname());
        let index_path = app_data_dir.join(index_filename());

        std::fs::create_dir_all(&recordings_dir)?;

        Ok(Self {
            app_data_dir,
            recordings_dir,
            index_path,
        })
    }

    pub fn load_or_rebuild_index(&self) -> Result<Vec<RecordingEntry>, StorageError> {
        if self.index_path.exists() {
            match self.load_index() {
                Ok(index) => return Ok(index.recordings),
                Err(err) => {
                    debug_log(&format!(
                        "failed to load index, rebuilding by scan: {err}"
                    ));
                }
            }
        }

        let rebuilt = self.rebuild_by_scanning()?;
        self.save_index(&rebuilt)?;
        Ok(rebuilt)
    }

    pub fn save_index(&self, recordings: &[RecordingEntry]) -> Result<(), StorageError> {
        let index = RecordingsIndex {
            version: 1,
            recordings: recordings.to_vec(),
        };

        let tmp = self.index_path.with_extension("json.tmp");
        {
            let f = File::create(&tmp)?;
            let mut w = BufWriter::new(f);
            serde_json::to_writer_pretty(&mut w, &index)?;
            w.write_all(b"\n")?;
            w.flush()?;
        }

        // Windows rename behavior can be picky; remove first if needed.
        if self.index_path.exists() {
            let _ = std::fs::remove_file(&self.index_path);
        }
        std::fs::rename(&tmp, &self.index_path)?;
        Ok(())
    }

    pub fn recordings_folder(&self) -> &Path {
        &self.recordings_dir
    }

    pub fn recording_path(&self, filename: &str) -> PathBuf {
        self.recordings_dir.join(filename)
    }

    fn load_index(&self) -> Result<RecordingsIndex, StorageError> {
        let f = File::open(&self.index_path)?;
        let r = BufReader::new(f);
        Ok(serde_json::from_reader(r)?)
    }

    fn rebuild_by_scanning(&self) -> Result<Vec<RecordingEntry>, StorageError> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.recordings_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("wav") {
                continue;
            }

            let filename = match path.file_name().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };

            let size_bytes = std::fs::metadata(&path)?.len();

            let (duration_sec, created_at) = self.read_wav_duration_and_created_at(&path, &filename)?;

            let id = filename.trim_end_matches(".wav").to_string();
            out.push(RecordingEntry {
                id,
                filename,
                created_at,
                duration_sec,
                size_bytes,
            });
        }

        // Newest first (lexicographic works with YYYY-MM-DD_HH-mm-ss).
        out.sort_by(|a, b| b.filename.cmp(&a.filename));
        Ok(out)
    }

    fn read_wav_duration_and_created_at(
        &self,
        path: &Path,
        filename: &str,
    ) -> Result<(f64, String), StorageError> {
        let created_at = filename_to_created_at(filename);

        let reader = hound::WavReader::open(path)?;
        let spec = reader.spec();
        let total_samples = reader.len() as f64;
        let channels = spec.channels.max(1) as f64;
        let sample_rate = spec.sample_rate.max(1) as f64;
        let frames = total_samples / channels;
        let duration_sec = frames / sample_rate;
        Ok((duration_sec, created_at))
    }
}

fn filename_to_created_at(filename: &str) -> String {
    let stem = filename.trim_end_matches(".wav");
    fallback_created_at_from_stem(stem)
}

fn fallback_created_at_from_stem(stem: &str) -> String {
    // Expected: YYYY-MM-DD_HH-mm-ss  =>  YYYY-MM-DDTHH:mm:ss
    let Some((d, t)) = stem.split_once('_') else {
        return stem.to_string();
    };
    format!("{d}T{}", t.replace('-', ":"))
}


