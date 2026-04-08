use std::time::Duration;

use lpdwise_core::types::{AudioChunk, Transcript, TranscriptSegment};
use reqwest::header::RETRY_AFTER;
use reqwest::multipart;
use serde::Deserialize;
use tracing::{debug, instrument, warn};

use crate::engine::{AsrEngine, AsrError};

const GROQ_TRANSCRIPTIONS_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const MODEL: &str = "whisper-large-v3";
const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const SAFE_MAX_UPLOAD_BYTES: u64 = 20 * 1024 * 1024;
const SAFE_MAX_CHUNK_DURATION: Duration = Duration::from_secs(10 * 60);

/// ASR engine backed by the Groq Whisper API.
pub struct GroqWhisperEngine {
    api_key: String,
    client: reqwest::Client,
    language: Option<String>,
}

/// Groq API verbose JSON response.
#[derive(Debug, Deserialize)]
struct GroqResponse {
    segments: Option<Vec<GroqSegment>>,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GroqSegment {
    start: f64,
    end: f64,
    text: String,
}

impl GroqWhisperEngine {
    pub fn new(api_key: String) -> Self {
        Self::with_language(api_key, None)
    }

    pub fn with_language(api_key: String, language: Option<&str>) -> Self {
        let client = reqwest::Client::new();
        let language = normalize_language(language);
        Self {
            api_key,
            client,
            language,
        }
    }

    /// Send a single audio file to the Groq Whisper API with retry on 429.
    #[instrument(skip(self, audio_path), fields(path = %audio_path.display()))]
    async fn call_api(&self, audio_path: &std::path::Path) -> Result<GroqResponse, AsrError> {
        let file_bytes = tokio::fs::read(audio_path).await.map_err(AsrError::Io)?;
        let file_size_bytes = file_bytes.len() as u64;
        if file_size_bytes > SAFE_MAX_UPLOAD_BYTES {
            return Err(AsrError::ApiRequest(format!(
                "audio chunk exceeds Groq safe upload budget: {} bytes > {} bytes",
                file_size_bytes, SAFE_MAX_UPLOAD_BYTES
            )));
        }

        let file_name = audio_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("audio.opus")
            .to_string();
        let mime = audio_mime(audio_path);

        let mut backoff = INITIAL_BACKOFF;

        for attempt in 0..=MAX_RETRIES {
            let part = multipart::Part::bytes(file_bytes.clone())
                .file_name(file_name.clone())
                .mime_str(mime)
                .map_err(|e| AsrError::ApiRequest(e.to_string()))?;

            let mut form = multipart::Form::new()
                .part("file", part)
                .text("model", MODEL)
                .text("response_format", "verbose_json");
            if let Some(language) = &self.language {
                form = form.text("language", language.clone());
            }

            let response = self
                .client
                .post(GROQ_TRANSCRIPTIONS_URL)
                .bearer_auth(&self.api_key)
                .multipart(form)
                .send()
                .await
                .map_err(|e| AsrError::ApiRequest(e.to_string()))?;

            let status = response.status();

            if status.is_success() {
                let body = response
                    .text()
                    .await
                    .map_err(|e| AsrError::ApiRequest(e.to_string()))?;

                let parsed: GroqResponse = serde_json::from_str(&body)
                    .map_err(|e| AsrError::Decode(format!("failed to parse Groq response: {e}")))?;

                return Ok(parsed);
            }

            if status.as_u16() == 429 {
                if attempt < MAX_RETRIES {
                    let retry_after = retry_after_delay(&response).unwrap_or(backoff);
                    warn!(
                        attempt = attempt + 1,
                        backoff_ms = retry_after.as_millis(),
                        "rate limited by Groq API, retrying"
                    );
                    tokio::time::sleep(retry_after).await;
                    backoff = retry_after.max(backoff) * 2;
                    continue;
                }
                return Err(AsrError::QuotaExceeded(
                    "Groq API rate limit exceeded after retries".into(),
                ));
            }

            let body = response.text().await.unwrap_or_default();
            return Err(AsrError::ApiRequest(format!(
                "Groq API returned {status}: {body}"
            )));
        }

        Err(AsrError::ApiRequest(
            "exhausted retries without success".into(),
        ))
    }
}

impl AsrEngine for GroqWhisperEngine {
    #[instrument(skip(self), fields(chunk_index = chunk.index))]
    async fn transcribe(&self, chunk: &AudioChunk) -> Result<Vec<TranscriptSegment>, AsrError> {
        let extracted_duration = chunk.end.saturating_sub(chunk.audio_start);
        if extracted_duration > SAFE_MAX_CHUNK_DURATION {
            return Err(AsrError::ApiRequest(format!(
                "audio chunk exceeds Groq safe context window: {:?} > {:?}",
                extracted_duration, SAFE_MAX_CHUNK_DURATION
            )));
        }

        let response = self.call_api(&chunk.path).await?;

        let segments = match response.segments {
            Some(segs) => segs
                .into_iter()
                .map(|s| TranscriptSegment {
                    text: s.text.trim().to_string(),
                    start: chunk.audio_start + Duration::from_secs_f64(s.start),
                    end: chunk.audio_start + Duration::from_secs_f64(s.end),
                })
                .collect(),
            None => {
                // Fallback: use the plain text field as a single segment
                let text = response.text.unwrap_or_default();
                if text.is_empty() {
                    return Ok(Vec::new());
                }
                vec![TranscriptSegment {
                    text: text.trim().to_string(),
                    start: chunk.audio_start,
                    end: chunk.end,
                }]
            }
        };

        debug!(segment_count = segments.len(), "transcribed chunk");
        Ok(segments)
    }
}

/// Transcribe multiple chunks sequentially, adjusting timestamps by chunk offset.
/// Returns a complete `Transcript` assembled from all chunks.
#[instrument(skip_all, fields(chunk_count = chunks.len()))]
pub async fn transcribe_chunks(
    chunks: &[AudioChunk],
    api_key: &str,
    language: Option<&str>,
) -> Result<Transcript, AsrError> {
    let engine = GroqWhisperEngine::with_language(api_key.to_string(), language);
    let mut all_segments = Vec::new();

    for chunk in chunks {
        let segments = engine.transcribe(chunk).await?;
        merge_chunk_segments(&mut all_segments, chunk, segments);
    }

    Ok(Transcript {
        segments: all_segments,
    })
}

pub fn merge_chunk_segments(
    all_segments: &mut Vec<TranscriptSegment>,
    chunk: &AudioChunk,
    segments: Vec<TranscriptSegment>,
) {
    if segments.is_empty() {
        return;
    }

    if chunk.audio_start < chunk.start {
        all_segments.retain(|segment| segment.start < chunk.audio_start);
    }
    all_segments.extend(segments);
}

fn normalize_language(language: Option<&str>) -> Option<String> {
    match language.map(str::trim).filter(|value| !value.is_empty()) {
        Some("auto") | None => None,
        Some(value) => Some(value.to_string()),
    }
}

fn audio_mime(audio_path: &std::path::Path) -> &'static str {
    match audio_path.extension().and_then(|ext| ext.to_str()) {
        Some("mp3" | "mpeg" | "mpga") => "audio/mpeg",
        Some("m4a" | "mp4") => "audio/mp4",
        Some("wav") => "audio/wav",
        Some("webm") => "audio/webm",
        Some("flac") => "audio/flac",
        Some("ogg" | "opus") => "audio/ogg",
        _ => "application/octet-stream",
    }
}

fn retry_after_delay(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(response_error_retry_delay)
}

fn response_error_retry_delay(value: &reqwest::header::HeaderValue) -> Option<Duration> {
    value
        .to_str()
        .ok()
        .and_then(|header| header.parse::<u64>().ok())
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_groq_response_parsing_with_segments() {
        let json = r#"{
            "text": "Hello world",
            "segments": [
                {"start": 0.0, "end": 1.5, "text": "Hello"},
                {"start": 1.5, "end": 3.0, "text": "world"}
            ]
        }"#;

        let parsed: GroqResponse = serde_json::from_str(json).unwrap();
        let segs = parsed.segments.unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "Hello");
    }

    #[test]
    fn test_groq_response_parsing_text_only() {
        let json = r#"{"text": "Hello world"}"#;

        let parsed: GroqResponse = serde_json::from_str(json).unwrap();
        assert!(parsed.segments.is_none());
        assert_eq!(parsed.text.as_deref(), Some("Hello world"));
    }

    #[test]
    fn test_timestamp_offset_applied() {
        let audio_start = Duration::from_secs(55);

        let groq_seg = GroqSegment {
            start: 5.0,
            end: 10.0,
            text: "test".into(),
        };

        let segment = TranscriptSegment {
            text: groq_seg.text.trim().to_string(),
            start: audio_start + Duration::from_secs_f64(groq_seg.start),
            end: audio_start + Duration::from_secs_f64(groq_seg.end),
        };

        assert_eq!(segment.start, Duration::from_secs(60));
        assert_eq!(segment.end, Duration::from_secs(65));
    }

    #[test]
    fn test_merge_chunk_segments_prefers_later_overlap() {
        let mut existing = vec![
            TranscriptSegment {
                text: "earlier".into(),
                start: Duration::from_secs(80),
                end: Duration::from_secs(95),
            },
            TranscriptSegment {
                text: "overlap".into(),
                start: Duration::from_secs(95),
                end: Duration::from_secs(105),
            },
        ];
        let chunk = AudioChunk {
            path: std::path::PathBuf::from("chunk.opus"),
            index: 1,
            audio_start: Duration::from_secs(90),
            start: Duration::from_secs(100),
            end: Duration::from_secs(180),
        };
        let next_segments = vec![TranscriptSegment {
            text: "later".into(),
            start: Duration::from_secs(92),
            end: Duration::from_secs(110),
        }];

        merge_chunk_segments(&mut existing, &chunk, next_segments);

        assert_eq!(existing.len(), 2);
        assert_eq!(existing[0].text, "earlier");
        assert_eq!(existing[1].text, "later");
    }

    #[test]
    fn test_normalize_language_ignores_auto() {
        assert_eq!(normalize_language(Some("auto")), None);
        assert_eq!(normalize_language(Some("zh")), Some("zh".into()));
    }

    #[test]
    fn test_audio_mime_from_extension() {
        assert_eq!(audio_mime(std::path::Path::new("sample.opus")), "audio/ogg");
        assert_eq!(audio_mime(std::path::Path::new("sample.mp3")), "audio/mpeg");
    }
}
