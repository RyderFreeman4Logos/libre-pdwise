use std::path::PathBuf;

use lpdwise_core::types::{AudioChunk, TranscriptSegment};

use crate::engine::{AsrEngine, AsrError};

/// ASR engine backed by sherpa-onnx for local on-device inference.
pub struct SherpaOnnxEngine {
    model_dir: PathBuf,
}

impl SherpaOnnxEngine {
    pub fn new(model_dir: PathBuf) -> Self {
        Self { model_dir }
    }
}

impl AsrEngine for SherpaOnnxEngine {
    async fn transcribe(&self, _chunk: &AudioChunk) -> Result<Vec<TranscriptSegment>, AsrError> {
        let _ = &self.model_dir;
        todo!("implement sherpa-onnx local transcription")
    }
}
