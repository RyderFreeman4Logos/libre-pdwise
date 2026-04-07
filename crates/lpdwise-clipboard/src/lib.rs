// Clipboard abstraction with feature-gated providers and universal stdio fallback.

#[cfg(feature = "desktop")]
mod arboard;
mod provider;
mod stdin;
mod stdout;
#[cfg(feature = "termux")]
mod termux;

#[cfg(feature = "desktop")]
pub use arboard::ArboardClipboard;
pub use provider::{auto_detect, ClipboardError, ClipboardProvider};
pub use stdin::StdinProvider;
pub use stdout::StdoutFallback;
#[cfg(feature = "termux")]
pub use termux::TermuxClipboard;
