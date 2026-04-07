//! Edge-case tests for prompt template rendering and assembly.

use std::time::Duration;

use lpdwise_core::language::Language;
use lpdwise_core::prompt::PromptTemplate;
use lpdwise_core::types::{Transcript, TranscriptSegment};

/// An empty transcript with no segments.
fn empty_transcript() -> Transcript {
    Transcript { segments: vec![] }
}

/// A transcript with a single short segment.
fn single_segment_transcript() -> Transcript {
    Transcript {
        segments: vec![TranscriptSegment {
            text: "Only segment".into(),
            start: Duration::from_secs(0),
            end: Duration::from_secs(2),
        }],
    }
}

/// A transcript with a very long text body.
fn long_transcript() -> Transcript {
    let long_text = "word ".repeat(10_000);
    Transcript {
        segments: vec![TranscriptSegment {
            text: long_text,
            start: Duration::from_secs(0),
            end: Duration::from_secs(3600),
        }],
    }
}

#[test]
fn test_empty_transcript_renders_without_panic() {
    let payload = PromptTemplate::Standard.render(&empty_transcript(), Language::Chinese);
    let assembled = payload.assemble();
    // Should still have context and instruction even with empty body
    assert!(assembled.contains("中文"));
    assert!(assembled.contains("结构化总结"));
}

#[test]
fn test_single_segment_transcript_renders() {
    let payload =
        PromptTemplate::Contrarian.render(&single_segment_transcript(), Language::English);
    let assembled = payload.assemble();
    assert!(assembled.contains("Only segment"));
    assert!(assembled.contains("反常识"));
}

#[test]
fn test_long_transcript_preserves_tail_placement() {
    let payload = PromptTemplate::Standard.render(&long_transcript(), Language::Chinese);
    let assembled = payload.assemble();

    // Tail-placement: instruction must appear after the body
    let body_pos = assembled.find("word word").unwrap();
    let instr_pos = assembled.find("结构化总结").unwrap();
    assert!(
        body_pos < instr_pos,
        "instruction must be placed after body for tail-placement strategy"
    );
}

#[test]
fn test_all_templates_render_all_languages() {
    let transcript = single_segment_transcript();
    let languages = [
        Language::Chinese,
        Language::English,
        Language::Japanese,
        Language::Auto,
    ];

    for template in PromptTemplate::ALL {
        for lang in &languages {
            let payload = template.render(&transcript, *lang);
            let assembled = payload.assemble();
            // Must always produce non-empty output
            assert!(
                !assembled.is_empty(),
                "template {:?} with language {:?} produced empty output",
                template,
                lang
            );
            // Must always contain the body text
            assert!(
                assembled.contains("Only segment"),
                "template {:?} with language {:?} missing body text",
                template,
                lang
            );
        }
    }
}

#[test]
fn test_pure_text_empty_segments_produces_empty_string() {
    let t = empty_transcript();
    assert_eq!(t.pure_text(), "");
}

#[test]
fn test_subtitle_text_empty_segments_produces_empty_string() {
    let t = empty_transcript();
    assert_eq!(t.subtitle_text(), "");
}
