use std::time::Duration;

use lpdwise_core::types::{AudioChunk, Transcript, TranscriptSegment};
use reqwest::multipart;
use serde::Deserialize;
use tracing::{debug, instrument, warn};

use crate::engine::{AsrEngine, AsrError};

const GROQ_TRANSCRIPTIONS_URL: &str =
    "https://api.groq.com/openai/v1/audio/transcriptions";
const MODEL: &str = "whisper-large-v3";
const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// ASR engine backed by the Groq Whisper API.
pub struct GroqWhisperEngine {
    api_key: String,
    client: reqwest::Client,
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
        let client = reqwest::Client::new();
        Self { api_key, client }
    }

    /// Send a single audio file to the Groq Whisper API with retry on 429.
    #[instrument(skip(self, audio_path), fields(path = %audio_path.display()))]
    async fn call_api(
        &self,
        audio_path: &std::path::Path,
    ) -> Result<GroqResponse, AsrError> {
        let file_bytes = tokio::fs::read(audio_path).await.map_err(AsrError::Io)?;

        let file_name = audio_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("audio.opus")
            .to_string();

        let mut backoff = INITIAL_BACKOFF;

        for attempt in 0..=MAX_RETRIES {
            let part = multipart::Part::bytes(file_bytes.clone())
                .file_name(file_name.clone())
                .mime_str("audio/ogg")
                .map_err(|e| AsrError::ApiRequest(e.to_string()))?;

            let form = multipart::Form::new()
                .part("file", part)
                .text("model", MODEL)
                .text("response_format", "verbose_json");

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

                let parsed: GroqResponse =
                    serde_json::from_str(&body).map_err(|e| {
                        AsrError::Decode(format!(
                            "failed to parse Groq response: {e}"
                        ))
                    })?;

                return Ok(parsed);
            }

            if status.as_u16() == 429 {
                if attempt < MAX_RETRIES {
                    warn!(
                        attempt = attempt + 1,
                        backoff_ms = backoff.as_millis(),
                        "rate limited by Groq API, retrying"
                    );
                    tokio::time::sleep(backoff).await;
                    backoff *= 2;
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
    async fn transcribe(
        &self,
        chunk: &AudioChunk,
    ) -> Result<Vec<TranscriptSegment>, AsrError> {
        let response = self.call_api(&chunk.path).await?;

        let segments = match response.segments {
            Some(segs) => segs
                .into_iter()
                .map(|s| TranscriptSegment {
                    text: s.text.trim().to_string(),
                    start: chunk.start + Duration::from_secs_f64(s.start),
                    end: chunk.start + Duration::from_secs_f64(s.end),
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
                    start: chunk.start,
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
) -> Result<Transcript, AsrError> {
    let engine = GroqWhisperEngine::new(api_key.to_string());
    let mut all_segments = Vec::new();

    for chunk in chunks {
        let segments = engine.transcribe(chunk).await?;
        all_segments.extend(segments);
    }

    Ok(Transcript {
        segments: all_segments,
    })
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
        let chunk_start = Duration::from_secs(60);

        let groq_seg = GroqSegment {
            start: 5.0,
            end: 10.0,
            text: "test".into(),
        };

        let segment = TranscriptSegment {
            text: groq_seg.text.trim().to_string(),
            start: chunk_start + Duration::from_secs_f64(groq_seg.start),
            end: chunk_start + Duration::from_secs_f64(groq_seg.end),
        };

        assert_eq!(segment.start, Duration::from_secs(65));
        assert_eq!(segment.end, Duration::from_secs(70));
    }
}
