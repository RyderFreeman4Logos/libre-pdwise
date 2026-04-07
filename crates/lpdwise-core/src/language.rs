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

/// An ASR engine type that can be recommended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineKind {
    /// Groq Whisper API (cloud).
    GroqWhisper,
    /// sherpa-onnx with SenseVoice model (local, optimized for Chinese).
    SherpaOnnxSenseVoice,
    /// sherpa-onnx with Whisper model (local, general purpose).
    SherpaOnnxWhisper,
}

/// A recommended engine with a priority ranking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineRecommendation {
    pub engine: EngineKind,
    /// Lower number = higher priority (1 = best choice).
    pub priority: u8,
    pub reason: String,
}

/// Recommend engines based on language, device capability, and Groq API availability.
///
/// Arguments are kept as primitives to avoid coupling core to device crate.
pub fn recommend_engines(
    language: Language,
    ram_mb: u64,
    groq_available: bool,
) -> Vec<EngineRecommendation> {
    let can_run_local = ram_mb >= 2048;

    match language {
        Language::Chinese => {
            let mut recs = Vec::with_capacity(3);

            // SenseVoice is best for Chinese
            if can_run_local {
                recs.push(EngineRecommendation {
                    engine: EngineKind::SherpaOnnxSenseVoice,
                    priority: 1,
                    reason: "SenseVoice is optimized for Chinese speech".into(),
                });
            }

            if groq_available {
                recs.push(EngineRecommendation {
                    engine: EngineKind::GroqWhisper,
                    priority: if can_run_local { 2 } else { 1 },
                    reason: "Groq Whisper API supports Chinese".into(),
                });
            }

            if can_run_local {
                recs.push(EngineRecommendation {
                    engine: EngineKind::SherpaOnnxWhisper,
                    priority: 3,
                    reason: "Whisper local as fallback for Chinese".into(),
                });
            }

            recs
        }
        Language::English | Language::Auto | Language::Japanese => {
            let mut recs = Vec::with_capacity(2);

            if groq_available {
                recs.push(EngineRecommendation {
                    engine: EngineKind::GroqWhisper,
                    priority: 1,
                    reason: "Groq Whisper API is fastest for English/general".into(),
                });
            }

            if can_run_local {
                recs.push(EngineRecommendation {
                    engine: EngineKind::SherpaOnnxWhisper,
                    priority: if groq_available { 2 } else { 1 },
                    reason: "Local Whisper as fallback".into(),
                });
            }

            recs
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bcp47_tags() {
        assert_eq!(Language::English.bcp47_tag(), "en");
        assert_eq!(Language::Chinese.bcp47_tag(), "zh");
        assert_eq!(Language::Japanese.bcp47_tag(), "ja");
        assert_eq!(Language::Auto.bcp47_tag(), "auto");
    }

    #[test]
    fn test_chinese_prefers_sensevoice() {
        let recs = recommend_engines(Language::Chinese, 4096, true);
        assert!(!recs.is_empty());
        assert_eq!(recs[0].engine, EngineKind::SherpaOnnxSenseVoice);
        assert_eq!(recs[0].priority, 1);
    }

    #[test]
    fn test_chinese_no_local_prefers_groq() {
        let recs = recommend_engines(Language::Chinese, 512, true);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].engine, EngineKind::GroqWhisper);
    }

    #[test]
    fn test_english_prefers_groq() {
        let recs = recommend_engines(Language::English, 4096, true);
        assert!(!recs.is_empty());
        assert_eq!(recs[0].engine, EngineKind::GroqWhisper);
        assert_eq!(recs[0].priority, 1);
    }

    #[test]
    fn test_english_no_groq_uses_local() {
        let recs = recommend_engines(Language::English, 4096, false);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].engine, EngineKind::SherpaOnnxWhisper);
        assert_eq!(recs[0].priority, 1);
    }

    #[test]
    fn test_no_resources_returns_empty() {
        let recs = recommend_engines(Language::English, 512, false);
        assert!(recs.is_empty());
    }

    #[test]
    fn test_auto_language_same_as_english() {
        let auto = recommend_engines(Language::Auto, 4096, true);
        let english = recommend_engines(Language::English, 4096, true);
        assert_eq!(auto.len(), english.len());
        for (a, e) in auto.iter().zip(english.iter()) {
            assert_eq!(a.engine, e.engine);
        }
    }
}
