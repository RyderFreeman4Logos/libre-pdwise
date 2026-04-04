use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Where media comes from: a local file or a remote URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputSource {
    File(PathBuf),
    Url(String),
}

/// A resolved media asset ready for processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAsset {
    pub source: InputSource,
    pub path: PathBuf,
    pub duration: Option<Duration>,
    pub size_bytes: Option<u64>,
}

/// A chunk of audio extracted from a larger media asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioChunk {
    pub path: PathBuf,
    pub index: usize,
    pub start: Duration,
    pub end: Duration,
}

/// A single segment of transcribed speech.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub text: String,
    pub start: Duration,
    pub end: Duration,
}

/// Complete transcript assembled from segments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub segments: Vec<TranscriptSegment>,
}

/// Payload sent to an LLM for knowledge extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptPayload {
    pub system: String,
    pub user: String,
}

/// A persisted record of a completed extraction run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveRecord {
    pub source: InputSource,
    pub transcript: Transcript,
    pub extracted_at: String,
}
