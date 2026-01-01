use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};
use time::format_description::FormatItem;
use time::macros::format_description;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no default input device available")]
    NoDefaultInputDevice,
    #[error("failed to query default input config: {0}")]
    DefaultInputConfig(#[from] cpal::DefaultStreamConfigError),
    #[error("failed to build input stream: {0}")]
    BuildStream(#[from] cpal::BuildStreamError),
    #[error("failed to start input stream: {0}")]
    PlayStream(#[from] cpal::PlayStreamError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("wav error: {0}")]
    Wav(#[from] hound::Error),
    #[error("unsupported input sample format")]
    UnsupportedSampleFormat,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FinishedRecording {
    pub filename: String,
    pub created_at: String,
    pub duration_sec: f64,
    pub size_bytes: u64,
}

fn debug_log(msg: &str) {
    if cfg!(debug_assertions) {
        eprintln!("[kiklet][audio] {msg}");
    }
}

fn filename_format() -> &'static [FormatItem<'static>] {
    format_description!("[year]-[month]-[day]_[hour]-[minute]-[second]")
}

fn created_at_format() -> &'static [FormatItem<'static>] {
    // RFC3339 format with Z (UTC): 2026-01-01T19:53:08Z
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z")
}

fn now_local_fallback_utc() -> time::OffsetDateTime {
    time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc())
}

fn f32_to_i16(s: f32) -> i16 {
    let clamped = s.clamp(-1.0, 1.0);
    (clamped * i16::MAX as f32) as i16
}

fn u16_to_i16(s: u16) -> i16 {
    (s as i32 - 32768) as i16
}

pub fn start_recording(recordings_dir: &Path) -> Result<RecordingSession, AudioError> {
    RecordingSession::start(recordings_dir)
}

pub fn stop_recording(active: RecordingSession) -> Result<FinishedRecording, AudioError> {
    active.stop()
}

pub struct RecordingSession {
    filename: String,
    created_at: String,
    stop_tx: mpsc::Sender<()>,
    join: Option<std::thread::JoinHandle<Result<FinishedRecording, AudioError>>>,
}

impl RecordingSession {
    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn created_at(&self) -> &str {
        &self.created_at
    }

    pub fn start(recordings_dir: &Path) -> Result<Self, AudioError> {
        std::fs::create_dir_all(recordings_dir)?;

        let now = now_local_fallback_utc();
        let stem = now
            .format(filename_format())
            .unwrap_or_else(|_| "recording".into());
        // Always use UTC for created_at to ensure consistent parsing (RFC3339 with Z)
        let now_utc = now.to_offset(time::UtcOffset::UTC);
        let created_at = match now_utc.format(created_at_format()) {
            Ok(s) => s,
            Err(_) => {
                // Fallback: construct RFC3339 manually
                format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                    now_utc.year(), now_utc.month() as u8, now_utc.day(),
                    now_utc.hour(), now_utc.minute(), now_utc.second())
            }
        };
        let filename = format!("{stem}.wav");
        let path = recordings_dir.join(&filename);

        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), AudioError>>();

        let filename_thread = filename.clone();
        let created_at_thread = created_at.clone();

        let join = std::thread::spawn(move || -> Result<FinishedRecording, AudioError> {
            let host = cpal::default_host();
            let device = host
                .default_input_device()
                .ok_or(AudioError::NoDefaultInputDevice)?;
            let supported = device.default_input_config()?;

            let sample_rate = supported.sample_rate().0;
            let channels_in = supported.channels().max(1) as usize;

            // Minimal, widely supported: 16-bit PCM WAV, mono (take first channel).
            let wav_spec = hound::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };

            let file = File::create(&path)?;
            let writer = hound::WavWriter::new(BufWriter::new(file), wav_spec)?;
            let writer = Arc::new(Mutex::new(Some(writer)));
            let samples_written = Arc::new(AtomicU64::new(0));

            let writer_cb = Arc::clone(&writer);
            let samples_written_cb = Arc::clone(&samples_written);

            let stream_config: cpal::StreamConfig = supported.clone().into();
            let err_fn = move |err| {
                debug_log(&format!("stream error: {err}"));
            };

            let stream = match supported.sample_format() {
                cpal::SampleFormat::I16 => device.build_input_stream(
                    &stream_config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let mut guard = match writer_cb.lock() {
                            Ok(g) => g,
                            Err(_) => return,
                        };
                        let Some(w) = guard.as_mut() else { return };

                        for i in (0..data.len()).step_by(channels_in) {
                            if let Ok(()) = w.write_sample(data[i]) {
                                samples_written_cb.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    },
                    err_fn,
                    None,
                )?,
                cpal::SampleFormat::U16 => device.build_input_stream(
                    &stream_config,
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        let mut guard = match writer_cb.lock() {
                            Ok(g) => g,
                            Err(_) => return,
                        };
                        let Some(w) = guard.as_mut() else { return };

                        for i in (0..data.len()).step_by(channels_in) {
                            if let Ok(()) = w.write_sample(u16_to_i16(data[i])) {
                                samples_written_cb.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    },
                    err_fn,
                    None,
                )?,
                cpal::SampleFormat::F32 => device.build_input_stream(
                    &stream_config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        let mut guard = match writer_cb.lock() {
                            Ok(g) => g,
                            Err(_) => return,
                        };
                        let Some(w) = guard.as_mut() else { return };

                        for i in (0..data.len()).step_by(channels_in) {
                            if let Ok(()) = w.write_sample(f32_to_i16(data[i])) {
                                samples_written_cb.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    },
                    err_fn,
                    None,
                )?,
                _ => return Err(AudioError::UnsupportedSampleFormat),
            };

            stream.play()?;

            let _ = ready_tx.send(Ok(()));

            // Block until stop is requested.
            let _ = stop_rx.recv();

            // Dropping the stream stops the callback; we only finalize after that.
            drop(stream);

            let samples = samples_written.load(Ordering::Relaxed);
            let duration_sec = if sample_rate == 0 {
                0.0
            } else {
                samples as f64 / sample_rate as f64
            };

            let mut guard = writer.lock().map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::Other, "wav writer lock poisoned")
            })?;
            if let Some(w) = guard.take() {
                w.finalize()?;
            }

            let size_bytes = std::fs::metadata(&path)?.len();

            Ok(FinishedRecording {
                filename: filename_thread,
                created_at: created_at_thread,
                duration_sec,
                size_bytes,
            })
        });

        // If initialization failed, return the exact error and avoid leaving a running thread.
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                filename,
                created_at,
                stop_tx,
                join: Some(join),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(AudioError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "recording thread failed to initialize",
            ))),
        }
    }

    pub fn stop(mut self) -> Result<FinishedRecording, AudioError> {
        let _ = self.stop_tx.send(());
        let join = self
            .join
            .take()
            .ok_or_else(|| AudioError::Io(std::io::Error::new(std::io::ErrorKind::Other, "missing join handle")))?;
        join.join().map_err(|_| {
            AudioError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "recording thread panicked",
            ))
        })?
    }
}


