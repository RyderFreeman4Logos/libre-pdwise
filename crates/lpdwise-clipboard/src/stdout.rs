use crate::provider::{ClipboardError, ClipboardProvider};

/// Fallback provider that writes text to stdout when no clipboard is available.
///
/// Reading is not supported — returns `ClipboardError::NotAvailable`.
pub struct StdoutFallback;

impl StdoutFallback {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StdoutFallback {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardProvider for StdoutFallback {
    fn get_content(&self) -> Result<String, ClipboardError> {
        Err(ClipboardError::NotAvailable)
    }

    fn set_content(&self, content: &str) -> Result<(), ClipboardError> {
        println!("{content}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stdout_fallback_get_returns_not_available() {
        let fb = StdoutFallback::new();
        let err = fb.get_content().unwrap_err();
        assert!(matches!(err, ClipboardError::NotAvailable));
    }

    #[test]
    fn test_stdout_fallback_set_succeeds() {
        let fb = StdoutFallback::new();
        fb.set_content("test output").unwrap();
    }
}
