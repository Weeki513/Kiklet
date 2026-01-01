use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum OpenAiError {
    #[error("missing OpenAI API key (set it in Settings)")]
    MissingApiKey,
    #[error("file not found")]
    FileNotFound,
    #[error("failed to read file: {0}")]
    Io(#[from] std::io::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("OpenAI API error: {0}")]
    Api(String),
}

#[derive(Debug, serde::Deserialize)]
struct WhisperResponse {
    text: String,
}

pub async fn transcribe_whisper(api_key: &str, file_path: &Path) -> Result<String, OpenAiError> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err(OpenAiError::MissingApiKey);
    }
    if !file_path.exists() {
        return Err(OpenAiError::FileNotFound);
    }

    let bytes = std::fs::read(file_path)?;
    let filename = file_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("audio.wav")
        .to_string();

    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(filename)
        .mime_str("audio/wav")
        .map_err(OpenAiError::Http)?;

    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .part("file", part);

    let client = reqwest::Client::builder()
        .user_agent("kiklet/0.1")
        .build()?;

    let resp = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .bearer_auth(key)
        .multipart(form)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(OpenAiError::Api(format!("{status}: {body}")));
    }

    let json: WhisperResponse = resp.json().await?;
    Ok(json.text.trim().to_string())
}


