use std::time::Duration;

use bytes::Bytes;
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
const DEFAULT_PROMPT_MAX_CHARS: usize = 256;
const PROMPT_TAIL_SEGMENTS: usize = 6;
const MAX_STITCH_SEGMENTS: usize = 6;
const MAX_STITCH_WORDS: usize = 24;
const MIN_STITCH_WORD_OVERLAP: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroqTranscriptionOptions {
    pub language: Option<String>,
    pub prompt_max_chars: usize,
}

impl GroqTranscriptionOptions {
    pub fn new(language: Option<&str>) -> Self {
        Self {
            language: normalize_language(language),
            prompt_max_chars: DEFAULT_PROMPT_MAX_CHARS,
        }
    }

    pub fn with_prompt_max_chars(mut self, prompt_max_chars: usize) -> Self {
        self.prompt_max_chars = prompt_max_chars;
        self
    }
}

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
    #[instrument(skip(self, audio_path, prompt), fields(path = %audio_path.display()))]
    async fn call_api(
        &self,
        audio_path: &std::path::Path,
        prompt: Option<&str>,
    ) -> Result<GroqResponse, AsrError> {
        let raw_bytes = tokio::fs::read(audio_path).await.map_err(AsrError::Io)?;
        let file_size_bytes = raw_bytes.len() as u64;
        if file_size_bytes > SAFE_MAX_UPLOAD_BYTES {
            return Err(AsrError::ApiRequest(format!(
                "audio chunk exceeds Groq safe upload budget: {} bytes > {} bytes",
                file_size_bytes, SAFE_MAX_UPLOAD_BYTES
            )));
        }
        let file_bytes = Bytes::from(raw_bytes);

        let file_name = audio_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("audio.opus")
            .to_string();
        let mime = audio_mime(audio_path);

        let mut backoff = INITIAL_BACKOFF;

        for attempt in 0..=MAX_RETRIES {
            let part = multipart::Part::stream(file_bytes.clone())
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
            if let Some(prompt) = prompt.map(str::trim).filter(|value| !value.is_empty()) {
                form = form.text("prompt", prompt.to_string());
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

    #[instrument(skip(self, prompt), fields(chunk_index = chunk.index))]
    async fn transcribe_with_prompt(
        &self,
        chunk: &AudioChunk,
        prompt: Option<&str>,
    ) -> Result<Vec<TranscriptSegment>, AsrError> {
        let extracted_duration = chunk.end.saturating_sub(chunk.audio_start);
        if extracted_duration > SAFE_MAX_CHUNK_DURATION {
            return Err(AsrError::ApiRequest(format!(
                "audio chunk exceeds Groq safe context window: {:?} > {:?}",
                extracted_duration, SAFE_MAX_CHUNK_DURATION
            )));
        }

        let response = self.call_api(&chunk.path, prompt).await?;

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

impl AsrEngine for GroqWhisperEngine {
    #[instrument(skip(self), fields(chunk_index = chunk.index))]
    async fn transcribe(&self, chunk: &AudioChunk) -> Result<Vec<TranscriptSegment>, AsrError> {
        self.transcribe_with_prompt(chunk, None).await
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
    transcribe_chunks_with_options(chunks, api_key, GroqTranscriptionOptions::new(language)).await
}

/// Transcribe multiple chunks sequentially with Groq-specific quality options.
#[instrument(skip_all, fields(chunk_count = chunks.len()))]
pub async fn transcribe_chunks_with_options(
    chunks: &[AudioChunk],
    api_key: &str,
    options: GroqTranscriptionOptions,
) -> Result<Transcript, AsrError> {
    let engine = GroqWhisperEngine::with_language(api_key.to_string(), options.language.as_deref());
    let mut all_segments = Vec::new();

    for chunk in chunks {
        let prompt = build_conditioning_prompt(&all_segments, chunk, options.prompt_max_chars);
        debug!(
            chunk_index = chunk.index,
            prompt_chars = prompt.as_ref().map_or(0, |value| value.chars().count()),
            "transcribing Groq chunk with sequential context"
        );
        let segments = engine
            .transcribe_with_prompt(chunk, prompt.as_deref())
            .await?;
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

    let stitched_segments = stitch_overlap_segments(all_segments, chunk, segments);
    all_segments.extend(stitched_segments);
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

fn build_conditioning_prompt(
    all_segments: &[TranscriptSegment],
    chunk: &AudioChunk,
    prompt_max_chars: usize,
) -> Option<String> {
    if prompt_max_chars == 0 {
        return None;
    }

    let prompt_text = all_segments
        .iter()
        .rev()
        .filter(|segment| segment.start < chunk.start)
        .take(PROMPT_TAIL_SEGMENTS)
        .map(|segment| segment.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();

    if prompt_text.is_empty() {
        return None;
    }

    let mut joined = prompt_text.into_iter().rev().collect::<Vec<_>>().join(" ");
    joined = clip_to_last_chars(&joined, prompt_max_chars);
    (!joined.is_empty()).then_some(joined)
}

fn stitch_overlap_segments(
    all_segments: &[TranscriptSegment],
    chunk: &AudioChunk,
    segments: Vec<TranscriptSegment>,
) -> Vec<TranscriptSegment> {
    if chunk.audio_start >= chunk.start || all_segments.is_empty() {
        return segments;
    }

    let overlap_segments = all_segments
        .iter()
        .filter(|segment| segment.end > chunk.audio_start)
        .rev()
        .take(MAX_STITCH_SEGMENTS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();

    if overlap_segments.is_empty() {
        return segments;
    }

    let segments = drop_exact_duplicate_prefix_segments(&overlap_segments, segments);
    trim_duplicate_prefix_words(&overlap_segments, chunk, segments)
}

fn drop_exact_duplicate_prefix_segments(
    overlap_segments: &[&TranscriptSegment],
    segments: Vec<TranscriptSegment>,
) -> Vec<TranscriptSegment> {
    let max_overlap = overlap_segments
        .len()
        .min(segments.len())
        .min(MAX_STITCH_SEGMENTS);
    let duplicate_segments = (1..=max_overlap)
        .rev()
        .find(|candidate_len| {
            overlap_segments[overlap_segments.len() - candidate_len..]
                .iter()
                .zip(segments.iter().take(*candidate_len))
                .all(|(left, right)| {
                    normalize_segment_text(&left.text) == normalize_segment_text(&right.text)
                })
        })
        .unwrap_or(0);

    segments.into_iter().skip(duplicate_segments).collect()
}

fn trim_duplicate_prefix_words(
    overlap_segments: &[&TranscriptSegment],
    chunk: &AudioChunk,
    segments: Vec<TranscriptSegment>,
) -> Vec<TranscriptSegment> {
    if segments.is_empty() {
        return segments;
    }

    let overlap_duration = chunk.start.saturating_sub(chunk.audio_start);
    if overlap_duration.is_zero() {
        return segments;
    }

    let boundary_limit = chunk.start + overlap_duration;
    let left_words = trailing_words(overlap_segments, MAX_STITCH_WORDS);
    let right_words = leading_words(
        segments
            .iter()
            .take_while(|segment| segment.start < boundary_limit),
        MAX_STITCH_WORDS,
    );
    let overlap_words = longest_word_overlap(&left_words, &right_words);
    if overlap_words < MIN_STITCH_WORD_OVERLAP {
        return segments;
    }

    trim_leading_words_from_segments(segments, overlap_words)
}

fn clip_to_last_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let total_chars = text.chars().count();
    if total_chars <= max_chars {
        return text.trim().to_string();
    }

    let start_char = total_chars.saturating_sub(max_chars);
    let start_byte = text
        .char_indices()
        .nth(start_char)
        .map(|(index, _)| index)
        .unwrap_or(0);
    let clipped = &text[start_byte..];
    let starts_on_word_boundary = start_byte == 0
        || text[..start_byte]
            .chars()
            .last()
            .is_some_and(char::is_whitespace);

    if starts_on_word_boundary {
        return clipped.trim().to_string();
    }

    if let Some(boundary) = clipped.find(char::is_whitespace) {
        let trimmed = clipped[boundary..].trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    clipped.trim().to_string()
}

fn normalize_segment_text(text: &str) -> String {
    normalize_words(text).join(" ")
}

fn normalize_token(token: &str) -> String {
    token
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|ch| ch.is_alphanumeric())
        .collect::<String>()
}

fn normalize_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(normalize_token)
        .filter(|word| !word.is_empty())
        .collect()
}

fn trailing_words(segments: &[&TranscriptSegment], max_words: usize) -> Vec<String> {
    let mut words = Vec::new();

    for segment in segments.iter().rev() {
        let segment_words = normalize_words(&segment.text);
        for word in segment_words.into_iter().rev() {
            words.push(word);
            if words.len() == max_words {
                words.reverse();
                return words;
            }
        }
    }

    words.reverse();
    words
}

fn leading_words<'a>(
    segments: impl Iterator<Item = &'a TranscriptSegment>,
    max_words: usize,
) -> Vec<String> {
    let mut words = Vec::new();

    for segment in segments {
        for word in normalize_words(&segment.text) {
            words.push(word);
            if words.len() == max_words {
                return words;
            }
        }
    }

    words
}

fn longest_word_overlap(left: &[String], right: &[String]) -> usize {
    let max_overlap = left.len().min(right.len()).min(MAX_STITCH_WORDS);
    (MIN_STITCH_WORD_OVERLAP..=max_overlap)
        .rev()
        .find(|candidate_len| left[left.len() - candidate_len..] == right[..*candidate_len])
        .unwrap_or(0)
}

fn trim_leading_words_from_segments(
    segments: Vec<TranscriptSegment>,
    mut words_to_trim: usize,
) -> Vec<TranscriptSegment> {
    let mut trimmed_segments = Vec::with_capacity(segments.len());

    for mut segment in segments {
        if words_to_trim == 0 {
            trimmed_segments.push(segment);
            continue;
        }

        let trimmed_text = trim_leading_words(&segment.text, words_to_trim);
        let removed_words = count_words(&segment.text).saturating_sub(count_words(&trimmed_text));
        words_to_trim = words_to_trim.saturating_sub(removed_words);

        if trimmed_text.is_empty() {
            continue;
        }

        segment.text = trimmed_text;
        trimmed_segments.push(segment);
    }

    trimmed_segments
}

fn trim_leading_words(text: &str, words_to_trim: usize) -> String {
    if words_to_trim == 0 {
        return text.trim().to_string();
    }

    let mut removed_words = 0usize;
    let mut token_start = None;

    for (idx, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if let Some(start) = token_start.take() {
                let token = &text[start..idx];
                if !normalize_token(token).is_empty() {
                    removed_words += 1;
                }
            }
            continue;
        }

        if token_start.is_none() {
            if removed_words >= words_to_trim {
                return text[idx..].trim().to_string();
            }
            token_start = Some(idx);
        }
    }

    if let Some(start) = token_start {
        if removed_words >= words_to_trim {
            return text[start..].trim().to_string();
        }
    }

    String::new()
}

fn count_words(text: &str) -> usize {
    normalize_words(text).len()
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
    fn test_merge_chunk_segments_drops_exact_duplicate_overlap_prefix() {
        let mut existing = vec![
            TranscriptSegment {
                text: "earlier".into(),
                start: Duration::from_secs(80),
                end: Duration::from_secs(95),
            },
            TranscriptSegment {
                text: "brown fox".into(),
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
        let next_segments = vec![
            TranscriptSegment {
                text: "brown fox".into(),
                start: Duration::from_secs(92),
                end: Duration::from_secs(103),
            },
            TranscriptSegment {
                text: "jumps".into(),
                start: Duration::from_secs(103),
                end: Duration::from_secs(110),
            },
        ];

        merge_chunk_segments(&mut existing, &chunk, next_segments);

        assert_eq!(existing.len(), 3);
        assert_eq!(existing[0].text, "earlier");
        assert_eq!(existing[1].text, "brown fox");
        assert_eq!(existing[2].text, "jumps");
    }

    #[test]
    fn test_merge_chunk_segments_trims_partial_overlap_words() {
        let mut existing = vec![TranscriptSegment {
            text: "brown fox jumps".into(),
            start: Duration::from_secs(95),
            end: Duration::from_secs(105),
        }];
        let chunk = AudioChunk {
            path: std::path::PathBuf::from("chunk.opus"),
            index: 1,
            audio_start: Duration::from_secs(90),
            start: Duration::from_secs(100),
            end: Duration::from_secs(180),
        };
        let next_segments = vec![TranscriptSegment {
            text: "fox jumps high".into(),
            start: Duration::from_secs(92),
            end: Duration::from_secs(110),
        }];

        merge_chunk_segments(&mut existing, &chunk, next_segments);

        assert_eq!(existing.len(), 2);
        assert_eq!(existing[0].text, "brown fox jumps");
        assert_eq!(existing[1].text, "high");
    }

    #[test]
    fn test_merge_chunk_segments_keeps_existing_overlap_when_new_chunk_is_distinct() {
        let mut existing = vec![TranscriptSegment {
            text: "cross-boundary".into(),
            start: Duration::from_secs(85),
            end: Duration::from_secs(101),
        }];
        let chunk = AudioChunk {
            path: std::path::PathBuf::from("chunk.opus"),
            index: 1,
            audio_start: Duration::from_secs(90),
            start: Duration::from_secs(100),
            end: Duration::from_secs(180),
        };
        let next_segments = vec![TranscriptSegment {
            text: "later".into(),
            start: Duration::from_secs(102),
            end: Duration::from_secs(110),
        }];

        merge_chunk_segments(&mut existing, &chunk, next_segments);

        assert_eq!(existing.len(), 2);
        assert_eq!(existing[0].text, "cross-boundary");
        assert_eq!(existing[1].text, "later");
    }

    #[test]
    fn test_build_conditioning_prompt_uses_recent_tail_text() {
        let all_segments = vec![
            TranscriptSegment {
                text: "alpha beta".into(),
                start: Duration::from_secs(0),
                end: Duration::from_secs(2),
            },
            TranscriptSegment {
                text: "gamma delta epsilon".into(),
                start: Duration::from_secs(2),
                end: Duration::from_secs(4),
            },
        ];
        let chunk = AudioChunk {
            path: std::path::PathBuf::from("chunk.opus"),
            index: 1,
            audio_start: Duration::from_secs(4),
            start: Duration::from_secs(5),
            end: Duration::from_secs(8),
        };

        let prompt = build_conditioning_prompt(&all_segments, &chunk, 14).unwrap();

        assert_eq!(prompt, "delta epsilon");
    }

    #[test]
    fn test_clip_to_last_chars_keeps_boundary_aligned_prefix() {
        let clipped = clip_to_last_chars("alpha beta gamma", 10);

        assert_eq!(clipped, "beta gamma");
    }

    #[test]
    fn test_clip_to_last_chars_drops_partial_word_prefix() {
        let clipped = clip_to_last_chars("alpha beta gamma", 9);

        assert_eq!(clipped, "gamma");
    }

    #[test]
    fn test_trim_leading_words_ignores_punctuation_only_prefix_tokens() {
        let trimmed = trim_leading_words("... hello world", 1);

        assert_eq!(trimmed, "world");
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
