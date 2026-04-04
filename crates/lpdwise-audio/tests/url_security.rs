//! Integration tests for URL validation security boundaries.
//!
//! These tests verify that the acquisition module correctly rejects
//! dangerous URL schemes and malformed inputs at the system boundary.

use lpdwise_audio::MediaAcquirer;
use lpdwise_core::types::InputSource;

/// Helper: attempt to acquire a URL source and expect an error message
/// containing the given substring.
async fn expect_url_rejected(url: &str, expected_fragment: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let acquirer = lpdwise_audio::YtDlpAcquirer::new(tmp.path().to_path_buf());

    let source = InputSource::Url(url.to_string());
    let result = acquirer.acquire(source).await;

    match result {
        Err(ref e) => {
            let msg = e.to_string();
            assert!(
                msg.contains(expected_fragment),
                "error for URL '{url}' should contain '{expected_fragment}', got: {msg}"
            );
        }
        Ok(_) => panic!("expected URL '{url}' to be rejected"),
    }
}

#[tokio::test]
async fn test_rejects_javascript_url() {
    expect_url_rejected("javascript:alert(1)", "invalid URL").await;
}

#[tokio::test]
async fn test_rejects_file_url() {
    expect_url_rejected("file:///etc/passwd", "invalid URL").await;
}

#[tokio::test]
async fn test_rejects_data_url() {
    expect_url_rejected("data:text/html,<h1>hi</h1>", "invalid URL").await;
}

#[tokio::test]
async fn test_rejects_blank_url() {
    expect_url_rejected("", "invalid URL").await;
}

#[tokio::test]
async fn test_rejects_whitespace_only_url() {
    expect_url_rejected("   ", "invalid URL").await;
}

#[tokio::test]
async fn test_rejects_url_with_spaces() {
    expect_url_rejected("http://example.com/path with spaces", "invalid URL").await;
}
