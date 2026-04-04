use std::path::PathBuf;

use lpdwise_clipboard::{auto_detect, ClipboardError};
use lpdwise_core::InputSource;
use tracing::{debug, info};

/// Resolve the input source from CLI arguments or clipboard.
///
/// - With argument: parse as URL (if starts with `https://` or `http://`) or local file path.
/// - Without argument: read clipboard, detect URL.
pub fn resolve_input(arg: Option<&str>) -> Result<InputSource, InputError> {
    match arg {
        Some(value) => resolve_from_argument(value),
        None => resolve_from_clipboard(),
    }
}

fn resolve_from_argument(value: &str) -> Result<InputSource, InputError> {
    if value.starts_with("http://") || value.starts_with("https://") {
        info!(url = %value, "input source: URL from argument");
        Ok(InputSource::Url(value.to_string()))
    } else {
        let path = PathBuf::from(value);
        if path.exists() {
            info!(path = %path.display(), "input source: local file from argument");
            Ok(InputSource::File(path))
        } else {
            Err(InputError::FileNotFound(path))
        }
    }
}

fn resolve_from_clipboard() -> Result<InputSource, InputError> {
    debug!("no argument provided, checking clipboard");
    let clipboard = auto_detect();
    let content = clipboard.get_content().map_err(InputError::Clipboard)?;
    let trimmed = content.trim();

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        info!(url = %trimmed, "input source: URL from clipboard");
        Ok(InputSource::Url(trimmed.to_string()))
    } else if trimmed.is_empty() {
        Err(InputError::NoInput)
    } else {
        Err(InputError::NoUrlInClipboard(trimmed.to_string()))
    }
}

/// Errors from input source resolution.
#[derive(Debug, thiserror::Error)]
pub enum InputError {
    #[error("file not found: {0}")]
    FileNotFound(PathBuf),

    #[error("clipboard error: {0}")]
    Clipboard(#[from] ClipboardError),

    #[error("no input provided and clipboard is empty")]
    NoInput,

    #[error("clipboard content is not a URL: {0}")]
    NoUrlInClipboard(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_https_url() {
        let result = resolve_from_argument("https://youtube.com/watch?v=abc").unwrap();
        assert!(matches!(result, InputSource::Url(url) if url.contains("youtube")));
    }

    #[test]
    fn test_resolve_http_url() {
        let result = resolve_from_argument("http://example.com/video.mp4").unwrap();
        assert!(matches!(result, InputSource::Url(_)));
    }

    #[test]
    fn test_resolve_nonexistent_file_errors() {
        let result = resolve_from_argument("/nonexistent/path/audio.opus");
        assert!(matches!(result, Err(InputError::FileNotFound(_))));
    }

    #[test]
    fn test_resolve_existing_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path_str = tmp.path().to_string_lossy().to_string();
        let result = resolve_from_argument(&path_str).unwrap();
        assert!(matches!(result, InputSource::File(_)));
    }
}
