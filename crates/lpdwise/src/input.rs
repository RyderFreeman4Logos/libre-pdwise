use std::path::PathBuf;

use lpdwise_clipboard::{auto_detect, ClipboardError};
use lpdwise_core::InputSource;
use tracing::{debug, info};

/// Resolve the input source from CLI arguments or clipboard.
///
/// - With argument: parse as URL (if starts with `https://` or `http://`) or local file path.
/// - Without argument: read clipboard/stdin fallback and parse URL or local file path.
pub(crate) fn resolve_input(arg: Option<&str>) -> Result<InputSource, InputError> {
    match arg {
        Some(value) => resolve_from_argument(value),
        None => resolve_from_clipboard(),
    }
}

fn resolve_from_argument(value: &str) -> Result<InputSource, InputError> {
    resolve_from_text(value, InputOrigin::Argument)
}

fn resolve_from_clipboard() -> Result<InputSource, InputError> {
    debug!("no argument provided, checking clipboard");
    let clipboard = auto_detect();
    let content = clipboard.get_content().map_err(InputError::Clipboard)?;
    resolve_from_text(&content, InputOrigin::ClipboardProvider)
}

fn resolve_from_text(value: &str, origin: InputOrigin) -> Result<InputSource, InputError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(InputError::NoInput);
    }

    if let Some(source) = resolve_url(trimmed) {
        info!(source = origin.label(), url = %trimmed, "input source: URL");
        return Ok(source);
    }

    let path = PathBuf::from(trimmed);
    if path.exists() {
        info!(
            source = origin.label(),
            path = %path.display(),
            "input source: local file"
        );
        Ok(InputSource::File(path))
    } else {
        Err(origin.invalid_input(trimmed, path))
    }
}

fn resolve_url(value: &str) -> Option<InputSource> {
    if value.starts_with("http://") || value.starts_with("https://") {
        Some(InputSource::Url(value.to_string()))
    } else {
        None
    }
}

#[derive(Clone, Copy)]
enum InputOrigin {
    Argument,
    ClipboardProvider,
}

impl InputOrigin {
    fn label(self) -> &'static str {
        match self {
            Self::Argument => "argument",
            Self::ClipboardProvider => "clipboard/stdin",
        }
    }

    fn invalid_input(self, value: &str, path: PathBuf) -> InputError {
        match self {
            Self::Argument => InputError::FileNotFound(path),
            Self::ClipboardProvider => InputError::InvalidClipboardInput(value.to_string()),
        }
    }
}

/// Errors from input source resolution.
#[derive(Debug, thiserror::Error)]
pub(crate) enum InputError {
    #[error("file not found: {0}")]
    FileNotFound(PathBuf),

    #[error("clipboard error: {0}")]
    Clipboard(#[from] ClipboardError),

    #[error("no input provided")]
    NoInput,

    #[error("clipboard/stdin content is not a URL or existing file: {0}")]
    InvalidClipboardInput(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file_in_tmp() -> tempfile::NamedTempFile {
        tempfile::NamedTempFile::new().unwrap()
    }

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
        let tmp = temp_file_in_tmp();
        let path_str = tmp.path().to_string_lossy().to_string();
        let result = resolve_from_argument(&path_str).unwrap();
        assert!(matches!(result, InputSource::File(_)));
    }

    #[test]
    fn test_resolve_existing_file_from_clipboard_provider_text() {
        let tmp = temp_file_in_tmp();
        let path_str = tmp.path().to_string_lossy().to_string();
        let result = resolve_from_text(&path_str, InputOrigin::ClipboardProvider).unwrap();
        assert!(matches!(result, InputSource::File(_)));
    }

    #[test]
    fn test_resolve_invalid_clipboard_provider_text_errors() {
        let result = resolve_from_text("not-a-url-or-file", InputOrigin::ClipboardProvider);
        assert!(matches!(
            result,
            Err(InputError::InvalidClipboardInput(value)) if value == "not-a-url-or-file"
        ));
    }
}
