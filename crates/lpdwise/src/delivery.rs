use lpdwise_archive::{ArchiveError, Archiver, GitArchiver};
use lpdwise_clipboard::ClipboardProvider;
use lpdwise_core::types::{ArchiveRecord, InputSource, PromptPayload, Transcript};
use tracing::{info, warn};

/// Size threshold (bytes) above which we warn about clipboard content length.
const CLIPBOARD_WARN_THRESHOLD: usize = 200 * 1024; // 200 KB

/// Deliver the assembled prompt to clipboard and optionally archive.
pub(crate) fn deliver(
    payload: &PromptPayload,
    clipboard: &dyn ClipboardProvider,
    source: &InputSource,
    transcript: &Transcript,
    archive_dir: Option<&std::path::Path>,
) -> Result<(), DeliveryError> {
    let content = payload.assemble();

    // Warn if content exceeds 200KB
    if content.len() > CLIPBOARD_WARN_THRESHOLD {
        warn!(
            size_kb = content.len() / 1024,
            "prompt exceeds 200KB — consider splitting for LLM consumption"
        );
    }

    // Write to clipboard
    clipboard
        .set_content(&content)
        .map_err(DeliveryError::Clipboard)?;
    info!(size_bytes = content.len(), "prompt copied to clipboard");

    // Archive if requested
    if let Some(dir) = archive_dir {
        let archiver = GitArchiver::init_or_open(dir).map_err(DeliveryError::Archive)?;
        let record = ArchiveRecord {
            source: source.clone(),
            transcript: transcript.clone(),
            extracted_at: now_iso8601(),
        };
        archiver.store(&record).map_err(DeliveryError::Archive)?;
        info!(path = %dir.display(), "transcript archived");
    }

    Ok(())
}

/// ISO 8601 timestamp for archive records.
fn now_iso8601() -> String {
    // Use simple std approach — avoid pulling in chrono/time crate for this
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    // Approximate UTC breakdown (no leap second handling — acceptable for logging)
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since epoch to Y-M-D (simplified civil calendar)
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Algorithm from Howard Hinnant's date library (public domain)
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

/// Errors from the delivery phase.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DeliveryError {
    #[error("clipboard delivery failed: {0}")]
    Clipboard(lpdwise_clipboard::ClipboardError),

    #[error("archive failed: {0}")]
    Archive(ArchiveError),
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use lpdwise_core::types::TranscriptSegment;

    use super::*;

    struct MockClipboard {
        content: RefCell<String>,
    }

    impl MockClipboard {
        fn new() -> Self {
            Self {
                content: RefCell::new(String::new()),
            }
        }

        fn get_stored(&self) -> String {
            self.content.borrow().clone()
        }
    }

    impl ClipboardProvider for MockClipboard {
        fn get_content(&self) -> Result<String, lpdwise_clipboard::ClipboardError> {
            Ok(self.content.borrow().clone())
        }

        fn set_content(&self, content: &str) -> Result<(), lpdwise_clipboard::ClipboardError> {
            *self.content.borrow_mut() = content.to_string();
            Ok(())
        }
    }

    fn sample_payload() -> PromptPayload {
        PromptPayload {
            context: "Context".into(),
            body: "Body text".into(),
            instruction: "Do something".into(),
        }
    }

    fn sample_transcript() -> Transcript {
        Transcript {
            segments: vec![TranscriptSegment {
                text: "Hello".into(),
                start: Duration::ZERO,
                end: Duration::from_secs(5),
            }],
        }
    }

    #[test]
    fn test_deliver_writes_to_clipboard() {
        let clipboard = MockClipboard::new();
        let payload = sample_payload();
        let source = InputSource::Url("https://example.com".into());

        deliver(&payload, &clipboard, &source, &sample_transcript(), None).unwrap();

        let stored = clipboard.get_stored();
        assert!(stored.contains("Context"));
        assert!(stored.contains("Body text"));
        assert!(stored.contains("Do something"));
    }

    #[test]
    fn test_deliver_with_archive() {
        let tmp = tempfile::tempdir().unwrap();
        let archive_dir = tmp.path().join("archive");

        let clipboard = MockClipboard::new();
        let payload = sample_payload();
        let source = InputSource::Url("https://example.com/video".into());

        deliver(
            &payload,
            &clipboard,
            &source,
            &sample_transcript(),
            Some(&archive_dir),
        )
        .unwrap();

        // Archive repo should exist with commits
        assert!(archive_dir.join(".git").exists());
    }

    #[test]
    fn test_deliver_no_archive_skips() {
        let clipboard = MockClipboard::new();
        let payload = sample_payload();
        let source = InputSource::Url("https://example.com".into());

        // No archive dir = skip archiving
        deliver(&payload, &clipboard, &source, &sample_transcript(), None).unwrap();
    }

    #[test]
    fn test_now_iso8601_format() {
        let ts = now_iso8601();
        // Should match YYYY-MM-DDTHH:MM:SSZ pattern
        assert_eq!(ts.len(), 20);
        assert!(ts.ends_with('Z'));
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
    }

    #[test]
    fn test_days_to_ymd_epoch() {
        // Unix epoch = 1970-01-01
        let (y, m, d) = days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn test_days_to_ymd_known_date() {
        // 2025-01-01 = 20089 days since epoch
        let (y, m, d) = days_to_ymd(20089);
        assert_eq!((y, m, d), (2025, 1, 1));
    }
}
