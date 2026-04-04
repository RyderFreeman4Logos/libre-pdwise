use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// lpdwise — audio/video knowledge extraction CLI.
#[derive(Parser, Debug)]
#[command(name = "lpdwise", version, about)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,

    /// Input source: file path or URL (when no subcommand is given).
    pub(crate) input: Option<String>,

    /// ASR engine to use.
    #[arg(long, default_value = "auto", value_enum)]
    pub(crate) engine: EngineArg,

    /// Source language of the media.
    #[arg(long, default_value = "auto", value_enum)]
    pub(crate) language: LanguageArg,

    /// Prompt template for knowledge extraction.
    #[arg(long, default_value = "standard", value_enum)]
    pub(crate) template: TemplateArg,

    /// Skip all interactive prompts.
    #[arg(long)]
    pub(crate) non_interactive: bool,

    /// Skip git archive after delivery.
    #[arg(long)]
    pub(crate) no_archive: bool,

    /// Custom directory for sherpa-onnx models.
    #[arg(long)]
    pub(crate) model_dir: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Check that required external tools are installed.
    Doctor,
}

/// CLI engine selection.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum EngineArg {
    /// Automatically select the best engine.
    Auto,
    /// Groq Whisper cloud API.
    Groq,
    /// sherpa-onnx local engine.
    Sherpa,
}

/// CLI language selection.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum LanguageArg {
    /// Auto-detect language.
    Auto,
    /// Chinese.
    Zh,
    /// English.
    En,
}

/// CLI template selection.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum TemplateArg {
    /// Structured summary with outline and key points.
    Standard,
    /// Extract contrarian, counterintuitive insights.
    Contrarian,
    /// Political-economic logic decomposition.
    Political,
    /// Full faithful translation to Chinese.
    Translation,
}

impl LanguageArg {
    /// Convert to core Language type.
    pub(crate) fn to_language(self) -> lpdwise_core::Language {
        match self {
            Self::Auto => lpdwise_core::Language::Auto,
            Self::Zh => lpdwise_core::Language::Chinese,
            Self::En => lpdwise_core::Language::English,
        }
    }
}

impl TemplateArg {
    /// Convert to core PromptTemplate type.
    pub(crate) fn to_template(self) -> lpdwise_core::PromptTemplate {
        match self {
            Self::Standard => lpdwise_core::PromptTemplate::Standard,
            Self::Contrarian => lpdwise_core::PromptTemplate::Contrarian,
            Self::Political => lpdwise_core::PromptTemplate::Political,
            Self::Translation => lpdwise_core::PromptTemplate::Translation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_arg_conversion() {
        assert_eq!(
            LanguageArg::Auto.to_language(),
            lpdwise_core::Language::Auto
        );
        assert_eq!(
            LanguageArg::Zh.to_language(),
            lpdwise_core::Language::Chinese
        );
        assert_eq!(
            LanguageArg::En.to_language(),
            lpdwise_core::Language::English
        );
    }

    #[test]
    fn test_template_arg_conversion() {
        // Verify all variants convert without panic
        let _ = TemplateArg::Standard.to_template();
        let _ = TemplateArg::Contrarian.to_template();
        let _ = TemplateArg::Political.to_template();
        let _ = TemplateArg::Translation.to_template();
    }

    #[test]
    fn test_cli_default_parsing() {
        // Verify default values parse correctly
        let cli = Cli::parse_from(["lpdwise", "https://example.com/video"]);
        assert!(matches!(cli.engine, EngineArg::Auto));
        assert!(matches!(cli.language, LanguageArg::Auto));
        assert!(matches!(cli.template, TemplateArg::Standard));
        assert!(!cli.non_interactive);
        assert!(!cli.no_archive);
        assert!(cli.model_dir.is_none());
    }

    #[test]
    fn test_cli_all_flags() {
        let cli = Cli::parse_from([
            "lpdwise",
            "--engine",
            "groq",
            "--language",
            "zh",
            "--template",
            "contrarian",
            "--non-interactive",
            "--no-archive",
            "--model-dir",
            "/tmp/models",
            "https://example.com",
        ]);
        assert!(matches!(cli.engine, EngineArg::Groq));
        assert!(matches!(cli.language, LanguageArg::Zh));
        assert!(matches!(cli.template, TemplateArg::Contrarian));
        assert!(cli.non_interactive);
        assert!(cli.no_archive);
        assert_eq!(cli.model_dir.unwrap(), PathBuf::from("/tmp/models"));
    }

    #[test]
    fn test_doctor_subcommand() {
        let cli = Cli::parse_from(["lpdwise", "doctor"]);
        assert!(matches!(cli.command, Some(Command::Doctor)));
    }
}
