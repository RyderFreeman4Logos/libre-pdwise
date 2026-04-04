use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Where media comes from: a local file or a remote URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputSource {
    File(PathBuf),
    Url(String),
}

/// A resolved media asset ready for processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAsset {
    pub source: InputSource,
    pub path: PathBuf,
    pub duration: Option<Duration>,
    pub size_bytes: Option<u64>,
}

/// A chunk of audio extracted from a larger media asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioChunk {
    pub path: PathBuf,
    pub index: usize,
    pub start: Duration,
    pub end: Duration,
}

/// A single segment of transcribed speech.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub text: String,
    pub start: Duration,
    pub end: Duration,
}

/// Complete transcript assembled from segments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub segments: Vec<TranscriptSegment>,
}

impl Transcript {
    /// Plain text without timestamps — segments joined by spaces.
    pub fn pure_text(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// SRT subtitle format with sequence numbers and time ranges.
    pub fn subtitle_text(&self) -> String {
        let mut buf = String::new();
        for (i, seg) in self.segments.iter().enumerate() {
            if i > 0 {
                buf.push('\n');
            }
            buf.push_str(&(i + 1).to_string());
            buf.push('\n');
            buf.push_str(&format_srt_time(seg.start));
            buf.push_str(" --> ");
            buf.push_str(&format_srt_time(seg.end));
            buf.push('\n');
            buf.push_str(&seg.text);
            buf.push('\n');
        }
        buf
    }
}

/// Format a Duration as SRT timestamp: HH:MM:SS,mmm
fn format_srt_time(d: Duration) -> String {
    let total_secs = d.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    let millis = d.subsec_millis();
    format!("{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
}

/// Payload sent to an LLM for knowledge extraction.
///
/// Three-part structure for tail-placement strategy:
/// the instruction is placed after the transcript body
/// to prevent "lost in the middle" attention degradation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptPayload {
    /// System-level context (role description, output format).
    pub context: String,
    /// Transcript body (the bulk of the content).
    pub body: String,
    /// Core instruction placed at the end (tail-placement).
    pub instruction: String,
}

impl PromptPayload {
    /// Assemble into a single user-facing prompt string.
    ///
    /// Layout: context → body → instruction (tail-placement).
    pub fn assemble(&self) -> String {
        format!("{}\n\n{}\n\n{}", self.context, self.body, self.instruction)
    }
}

/// A persisted record of a completed extraction run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveRecord {
    pub source: InputSource,
    pub transcript: Transcript,
    pub extracted_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_segments() -> Vec<TranscriptSegment> {
        vec![
            TranscriptSegment {
                text: "Hello world".into(),
                start: Duration::from_millis(0),
                end: Duration::from_millis(3500),
            },
            TranscriptSegment {
                text: "Second segment".into(),
                start: Duration::from_millis(3500),
                end: Duration::from_millis(7200),
            },
        ]
    }

    #[test]
    fn test_pure_text_joins_with_space() {
        let t = Transcript {
            segments: make_segments(),
        };
        assert_eq!(t.pure_text(), "Hello world Second segment");
    }

    #[test]
    fn test_pure_text_empty_transcript() {
        let t = Transcript { segments: vec![] };
        assert_eq!(t.pure_text(), "");
    }

    #[test]
    fn test_subtitle_text_srt_format() {
        let t = Transcript {
            segments: make_segments(),
        };
        let srt = t.subtitle_text();
        assert!(srt.contains("1\n00:00:00,000 --> 00:00:03,500\nHello world\n"));
        assert!(srt.contains("2\n00:00:03,500 --> 00:00:07,200\nSecond segment\n"));
    }

    #[test]
    fn test_subtitle_text_empty_transcript() {
        let t = Transcript { segments: vec![] };
        assert_eq!(t.subtitle_text(), "");
    }

    #[test]
    fn test_format_srt_time_hours() {
        let d = Duration::from_secs(3661) + Duration::from_millis(42);
        assert_eq!(format_srt_time(d), "01:01:01,042");
    }

    #[test]
    fn test_prompt_payload_assemble_order() {
        let p = PromptPayload {
            context: "CTX".into(),
            body: "BODY".into(),
            instruction: "INSTR".into(),
        };
        let assembled = p.assemble();
        assert_eq!(assembled, "CTX\n\nBODY\n\nINSTR");
    }
}
