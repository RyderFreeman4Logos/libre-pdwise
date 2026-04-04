// ASR engine trait with Groq Whisper API and sherpa-onnx implementations.

pub mod engine;
pub mod groq;
pub mod sherpa;

pub use engine::{AsrEngine, AsrError};
pub use groq::GroqWhisperEngine;
pub use sherpa::SherpaOnnxEngine;
