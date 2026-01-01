use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use tauri::Manager;
use chrono::TimeZone;

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

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedDelete {
    pub filename: String,
    pub error: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearAllReport {
    pub deleted_count: usize,
    pub files_on_disk_before: usize,
    pub index_count_before: usize,
    pub failed_deletes: Vec<FailedDelete>,
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

/// Get the recordings directory path. This is the SINGLE SOURCE OF TRUTH for where recordings are stored.
/// Creates the directory if it doesn't exist.
pub fn recordings_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, StorageError> {
    let app_data_dir = app.path().app_data_dir()?;
    let recordings_dir = app_data_dir.join(recordings_dirname());
    std::fs::create_dir_all(&recordings_dir)?;
    Ok(recordings_dir)
}

impl Storage {
    pub fn new(app: &tauri::AppHandle) -> Result<Self, StorageError> {
        let app_data_dir = app.path().app_data_dir()?;
        let recordings_dir = recordings_dir(app)?;
        let index_path = app_data_dir.join(index_filename());

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

    pub fn purge_old_recordings(&self, app: &tauri::AppHandle, days: u32) -> Result<(usize, usize), StorageError> {
        eprintln!("[kiklet][storage] purge_old_recordings START days={}", days);
        
        // Use SINGLE SOURCE OF TRUTH for recordings_dir
        let recordings_dir = recordings_dir(app)?;
        eprintln!("[kiklet][storage] recordings_dir={:?}", recordings_dir);
        eprintln!("[kiklet][storage] recordings_json={:?}", self.index_path);
        
        // Load or rebuild index
        let recordings = self.load_or_rebuild_index()?;
        let total_count = recordings.len();
        eprintln!("[kiklet][storage] index BEFORE: {} recordings", total_count);
        
        // Count actual files on disk
        let mut files_on_disk = 0;
        if let Ok(entries) = std::fs::read_dir(recordings_dir.as_path()) {
            for entry in entries.flatten() {
                if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                    if ext == "wav" {
                        files_on_disk += 1;
                    }
                }
            }
        }
        eprintln!("[kiklet][storage] files_on_disk: {} .wav files", files_on_disk);
        
        // Calculate cutoff time
        let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);
        eprintln!("[kiklet][storage] cutoff_time: {} (UTC)", cutoff);
        
        let mut deleted_count = 0;
        let mut kept_count = 0;
        let mut to_keep = Vec::new();
        
        for entry in recordings.iter() {
            // STRICT: determine age ONLY from recordings.json "created_at".
            // Renames/mtime must NOT affect purge.
            let created = match chrono::DateTime::parse_from_rfc3339(&entry.created_at) {
                Ok(dt) => dt.with_timezone(&chrono::Utc),
                Err(_) => {
                    // Legacy format without timezone (e.g. "2026-01-01T23:53:08"):
                    // treat as LOCAL time and convert to UTC when possible.
                    match chrono::NaiveDateTime::parse_from_str(&entry.created_at, "%Y-%m-%dT%H:%M:%S") {
                        Ok(naive) => {
                            let local_dt = chrono::Local.from_local_datetime(&naive);
                            if let chrono::LocalResult::Single(ldt) = local_dt {
                                eprintln!(
                                    "[kiklet][storage] legacy created_at (no tz) for {} -> treating as LOCAL, converting to UTC",
                                    entry.filename
                                );
                                ldt.with_timezone(&chrono::Utc)
                            } else {
                                eprintln!(
                                    "[kiklet][storage] legacy created_at (no tz) ambiguous/invalid local time for {} -> treating as UTC",
                                    entry.filename
                                );
                                naive.and_utc()
                            }
                        }
                        Err(_) => {
                            // FAIL-CLOSED: invalid created_at => delete
                            eprintln!(
                                "[kiklet][storage] invalid created_at '{}' for {} -> delete (fail-closed)",
                                entry.created_at,
                                entry.filename
                            );
                            chrono::DateTime::from_timestamp(0, 0)
                                .unwrap_or_else(|| chrono::Utc::now() - chrono::Duration::days(365))
                                .with_timezone(&chrono::Utc)
                        }
                    }
                }
            };
            
            if created < cutoff {
                eprintln!("[kiklet][storage] OLD: {} (created={}, cutoff={})", entry.filename, created, cutoff);
                // Delete WAV file
                let wav_path = recordings_dir.join(&entry.filename);
                eprintln!("[kiklet][storage] purge delete: {:?} exists={}", wav_path, wav_path.exists());
                if wav_path.exists() {
                    match std::fs::remove_file(&wav_path) {
                        Ok(()) => {
                            deleted_count += 1;
                            eprintln!("[kiklet][storage] delete ok: {}", entry.filename);
                        }
                        Err(e) => {
                            eprintln!("[kiklet][storage] delete failed: {} err={}", entry.filename, e);
                            eprintln!("[kiklet][storage] removing from index anyway (fail-open for index)");
                            deleted_count += 1;
                        }
                    }
                } else {
                    eprintln!("[kiklet][storage] missing (already gone): {}, removing from index", entry.filename);
                    deleted_count += 1; // File missing, remove from index
                }
            } else {
                kept_count += 1;
                to_keep.push(entry.clone());
            }
        }
        
        // Save updated index
        self.save_index(&to_keep)?;
        eprintln!("[kiklet][storage] purge complete: deleted={}, kept={}", deleted_count, kept_count);
        println!("[kiklet][storage] purge complete: deleted={}, kept={}", deleted_count, kept_count);
        eprintln!("[kiklet][storage] index AFTER: {} recordings", to_keep.len());
        
        Ok((deleted_count, kept_count))
    }

    pub fn clear_all_recordings(&self, app: &tauri::AppHandle) -> Result<ClearAllReport, StorageError> {
        eprintln!("[kiklet][storage] clear_all_recordings START");
        
        // Use SINGLE SOURCE OF TRUTH for recordings_dir
        let recordings_dir = recordings_dir(app)?;
        eprintln!("[kiklet][storage] recordings_dir={:?}", recordings_dir);
        eprintln!("[kiklet][storage] recordings_json={:?}", self.index_path);
        
        // Load or rebuild index to get list of files
        let recordings = self.load_or_rebuild_index()?;
        let total_count = recordings.len();
        eprintln!("[kiklet][storage] index BEFORE: {} recordings", total_count);
        
        // Also scan disk for any .wav files not in index
        let mut files_on_disk = Vec::new();
        if let Ok(entries) = std::fs::read_dir(recordings_dir.as_path()) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if ext == "wav" {
                        if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
                            files_on_disk.push(filename.to_string());
                        }
                    }
                }
            }
        }
        eprintln!(
            "[kiklet][storage] files_on_disk_before: {} .wav files",
            files_on_disk.len()
        );
        
        let files_on_disk_before = files_on_disk.len();
        let index_count_before = total_count;
        let mut deleted_count = 0usize;
        let mut failed_deletes: Vec<FailedDelete> = Vec::new();
        
        // Delete all WAV files from index
        for entry in recordings.iter() {
            let wav_path = recordings_dir.join(&entry.filename);
            eprintln!("[kiklet][storage] attempting to delete: {:?}", wav_path);
            if wav_path.exists() {
                match std::fs::remove_file(&wav_path) {
                    Ok(()) => {
                        deleted_count += 1;
                        eprintln!("[kiklet][storage] delete ok: {}", entry.filename);
                    }
                    Err(e) => {
                        eprintln!("[kiklet][storage] delete failed: {} err={}", entry.filename, e);
                        failed_deletes.push(FailedDelete {
                            filename: entry.filename.clone(),
                            error: e.to_string(),
                        });
                    }
                }
            } else {
                eprintln!("[kiklet][storage] missing (already gone): {}", entry.filename);
            }
        }
        
        // Delete any .wav files on disk that weren't in index
        for filename in files_on_disk.iter() {
            if !recordings.iter().any(|e| e.filename == *filename) {
                let wav_path = recordings_dir.join(filename);
                eprintln!("[kiklet][storage] deleting orphaned file: {}", filename);
                match std::fs::remove_file(&wav_path) {
                    Ok(()) => {
                        deleted_count += 1;
                        eprintln!("[kiklet][storage] delete ok (orphaned): {}", filename);
                    }
                    Err(e) => {
                        eprintln!("[kiklet][storage] delete failed (orphaned): {} err={}", filename, e);
                        failed_deletes.push(FailedDelete {
                            filename: filename.clone(),
                            error: e.to_string(),
                        });
                    }
                }
            }
        }
        
        // Clear index
        self.save_index(&[])?;
        eprintln!(
            "[kiklet][storage] clear done deleted={} failed={}",
            deleted_count,
            failed_deletes.len()
        );
        println!(
            "[kiklet][storage] clear done deleted={} failed={}",
            deleted_count,
            failed_deletes.len()
        );
        eprintln!("[kiklet][storage] index AFTER: 0 recordings (cleared)");
        
        // NOTE: We do NOT fail the whole command on per-file delete errors.
        // We return a report so the UI can show errors deterministically.
        Ok(ClearAllReport {
            deleted_count,
            files_on_disk_before,
            index_count_before,
            failed_deletes,
        })
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


