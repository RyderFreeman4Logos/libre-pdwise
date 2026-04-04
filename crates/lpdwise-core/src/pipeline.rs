use crate::types::{AudioChunk, MediaAsset, Transcript};

/// Stages of the extraction pipeline.
///
/// Each stage transforms data from the previous stage's output type
/// into the next stage's input type.
pub trait Pipeline {
    /// Acquire media and produce a resolved asset.
    fn acquire(&self) -> impl std::future::Future<Output = Result<MediaAsset, PipelineError>>;

    /// Split a media asset into processable audio chunks.
    fn chunk(
        &self,
        asset: &MediaAsset,
    ) -> impl std::future::Future<Output = Result<Vec<AudioChunk>, PipelineError>>;

    /// Transcribe audio chunks into a complete transcript.
    fn transcribe(
        &self,
        chunks: &[AudioChunk],
    ) -> impl std::future::Future<Output = Result<Transcript, PipelineError>>;
}

/// Errors that can occur during pipeline execution.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("acquisition failed: {0}")]
    Acquisition(String),

    #[error("chunking failed: {0}")]
    Chunking(String),

    #[error("transcription failed: {0}")]
    Transcription(String),
}
