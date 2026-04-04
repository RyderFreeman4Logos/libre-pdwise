/// Abstraction over clipboard read/write operations.
pub trait ClipboardProvider {
    /// Read text from the system clipboard.
    fn get_content(&self) -> Result<String, ClipboardError>;

    /// Write text to the system clipboard.
    fn set_content(&self, content: &str) -> Result<(), ClipboardError>;
}

/// Errors from clipboard operations.
#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error("clipboard access denied: {0}")]
    AccessDenied(String),

    #[error("clipboard provider unavailable: {0}")]
    Unavailable(String),

    #[error("clipboard read not supported by this provider")]
    NotAvailable,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Detect the best clipboard provider for the current environment.
///
/// Priority: Termux → Desktop (arboard) → stdout fallback.
pub fn auto_detect() -> Box<dyn ClipboardProvider> {
    if std::env::var("TERMUX_VERSION").is_ok() {
        tracing::debug!("detected Termux environment, using TermuxClipboard");
        return Box::new(crate::TermuxClipboard::new());
    }

    if std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok() {
        tracing::debug!("detected display server, trying arboard");
        match crate::ArboardClipboard::new() {
            Ok(clipboard) => return Box::new(clipboard),
            Err(e) => {
                tracing::warn!("arboard init failed: {e}, falling back to stdout");
            }
        }
    }

    tracing::debug!("no clipboard backend available, using stdout fallback");
    Box::new(crate::StdoutFallback::new())
}
