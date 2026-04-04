// Core types, pipeline traits, language detection, prompt templates, config.

pub mod config;
pub mod language;
pub mod pipeline;
pub mod prompt;
pub mod types;

pub use config::{load_config, AppConfig, ConfigError};
pub use language::Language;
pub use pipeline::{Pipeline, PipelineError};
pub use prompt::PromptTemplate;
pub use types::{
    ArchiveRecord, AudioChunk, InputSource, MediaAsset, PromptPayload, Transcript,
    TranscriptSegment,
};
