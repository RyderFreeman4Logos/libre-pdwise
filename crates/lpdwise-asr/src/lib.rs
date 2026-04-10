// ASR engine trait with Groq Whisper API and sherpa-onnx implementations.

pub mod engine;
pub mod groq;
pub mod model;
pub mod sherpa;

pub use engine::{AsrEngine, AsrError};
pub use groq::{
    merge_chunk_segments, transcribe_chunks, transcribe_chunks_with_options,
    GroqTranscriptionOptions, GroqWhisperEngine,
};
pub use model::{download_model, ModelError};
pub use sherpa::SherpaOnnxEngine;
