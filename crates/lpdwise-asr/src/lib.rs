// ASR engine trait with Groq Whisper API and sherpa-onnx implementations.

pub mod engine;
pub mod groq;
pub mod model;
pub mod sherpa;

pub use engine::{AsrEngine, AsrError};
pub use groq::{transcribe_chunks, GroqWhisperEngine};
pub use model::{download_model, ModelError};
pub use sherpa::SherpaOnnxEngine;
