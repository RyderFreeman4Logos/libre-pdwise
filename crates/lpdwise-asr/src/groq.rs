use lpdwise_core::types::{AudioChunk, TranscriptSegment};

use crate::engine::{AsrEngine, AsrError};

/// ASR engine backed by the Groq Whisper API.
pub struct GroqWhisperEngine {
    api_key: String,
}

impl GroqWhisperEngine {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl AsrEngine for GroqWhisperEngine {
    async fn transcribe(&self, _chunk: &AudioChunk) -> Result<Vec<TranscriptSegment>, AsrError> {
        let _ = &self.api_key;
        todo!("implement Groq Whisper API transcription")
    }
}
