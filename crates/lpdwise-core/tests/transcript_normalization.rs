//! Tests for transcript normalization edge cases.

use std::time::Duration;

use lpdwise_core::types::{Transcript, TranscriptSegment};

/// Empty segments list should produce empty strings.
#[test]
fn test_empty_segments_pure_text() {
    let t = Transcript { segments: vec![] };
    assert_eq!(t.pure_text(), "");
}

#[test]
fn test_empty_segments_subtitle_text() {
    let t = Transcript { segments: vec![] };
    assert_eq!(t.subtitle_text(), "");
}

/// Single segment round-trip through pure_text and subtitle_text.
#[test]
fn test_single_segment_pure_text() {
    let t = Transcript {
        segments: vec![TranscriptSegment {
            text: "Hello".into(),
            start: Duration::from_secs(0),
            end: Duration::from_secs(1),
        }],
    };
    assert_eq!(t.pure_text(), "Hello");
}

#[test]
fn test_single_segment_subtitle_text() {
    let t = Transcript {
        segments: vec![TranscriptSegment {
            text: "Hello".into(),
            start: Duration::from_millis(500),
            end: Duration::from_millis(1500),
        }],
    };
    let srt = t.subtitle_text();
    assert!(srt.contains("1\n"));
    assert!(srt.contains("00:00:00,500 --> 00:00:01,500"));
    assert!(srt.contains("Hello"));
}

/// Long text segment should not be truncated.
#[test]
fn test_long_text_segment_preserved() {
    let long_text = "a".repeat(50_000);
    let t = Transcript {
        segments: vec![TranscriptSegment {
            text: long_text.clone(),
            start: Duration::from_secs(0),
            end: Duration::from_secs(3600),
        }],
    };
    assert_eq!(t.pure_text(), long_text);
}

/// Segments with empty text should still join correctly.
#[test]
fn test_segments_with_empty_text() {
    let t = Transcript {
        segments: vec![
            TranscriptSegment {
                text: "".into(),
                start: Duration::from_secs(0),
                end: Duration::from_secs(1),
            },
            TranscriptSegment {
                text: "world".into(),
                start: Duration::from_secs(1),
                end: Duration::from_secs(2),
            },
        ],
    };
    assert_eq!(t.pure_text(), " world");
}

/// SRT format should handle zero-duration segments.
#[test]
fn test_zero_duration_segment_srt() {
    let t = Transcript {
        segments: vec![TranscriptSegment {
            text: "Instant".into(),
            start: Duration::from_secs(10),
            end: Duration::from_secs(10),
        }],
    };
    let srt = t.subtitle_text();
    assert!(srt.contains("00:00:10,000 --> 00:00:10,000"));
}

/// Verify SRT timestamp formatting for large durations (hours).
#[test]
fn test_srt_timestamp_hours() {
    let t = Transcript {
        segments: vec![TranscriptSegment {
            text: "Late segment".into(),
            start: Duration::from_secs(7261) + Duration::from_millis(42),
            end: Duration::from_secs(7300),
        }],
    };
    let srt = t.subtitle_text();
    assert!(srt.contains("02:01:01,042"));
}
