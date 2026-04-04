use serde::{Deserialize, Serialize};

/// Supported languages for transcription and prompt rendering.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    English,
    Chinese,
    Japanese,
    #[default]
    Auto,
}

impl Language {
    /// BCP-47 language tag for ASR engines.
    pub fn bcp47_tag(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Chinese => "zh",
            Self::Japanese => "ja",
            Self::Auto => "auto",
        }
    }
}
