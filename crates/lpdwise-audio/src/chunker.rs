use std::time::Duration;

use lpdwise_core::types::{AudioChunk, MediaAsset};

/// A detected gap of silence in the audio stream.
#[derive(Debug, Clone)]
pub struct SilenceGap {
    pub start: Duration,
    pub end: Duration,
}

/// Splits a media asset into audio chunks using adaptive silence-based cutting.
///
/// The algorithm detects silence gaps and ensures each chunk
/// meets the minimum duration constraint.
pub fn adaptive_chunk(
    _asset: &MediaAsset,
    _min_chunk_duration: Duration,
) -> Result<Vec<AudioChunk>, ChunkerError> {
    todo!("implement adaptive silence-based chunking")
}

/// Errors from audio chunking.
#[derive(Debug, thiserror::Error)]
pub enum ChunkerError {
    #[error("ffmpeg silence detection failed: {0}")]
    SilenceDetection(String),

    #[error("no audio data in asset")]
    EmptyAudio,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
