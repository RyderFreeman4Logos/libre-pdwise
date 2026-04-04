use lpdwise_core::types::{AudioChunk, TranscriptSegment};

/// Common interface for automatic speech recognition engines.
pub trait AsrEngine {
    /// Transcribe a single audio chunk into transcript segments.
    fn transcribe(
        &self,
        chunk: &AudioChunk,
    ) -> impl std::future::Future<Output = Result<Vec<TranscriptSegment>, AsrError>>;
}

/// Errors from ASR processing.
#[derive(Debug, thiserror::Error)]
pub enum AsrError {
    #[error("api request failed: {0}")]
    ApiRequest(String),

    #[error("api quota exceeded: {0}")]
    QuotaExceeded(String),

    #[error("model loading failed: {0}")]
    ModelLoad(String),

    #[error("decoding failed: {0}")]
    Decode(String),

    #[error("engine not available: {0}")]
    NotAvailable(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
