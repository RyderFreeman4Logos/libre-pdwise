use crate::language::Language;
use crate::types::PromptPayload;

/// Built-in prompt templates for knowledge extraction.
#[derive(Debug, Clone, Copy)]
pub enum PromptTemplate {
    /// Summarize the transcript into key points.
    Summary,
    /// Extract structured notes from the transcript.
    Notes,
}

impl PromptTemplate {
    /// Render this template into a concrete prompt payload.
    pub fn render(self, _transcript_text: &str, _language: Language) -> PromptPayload {
        todo!("render prompt template")
    }
}
