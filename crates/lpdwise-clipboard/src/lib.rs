// Clipboard abstraction: arboard (desktop) + termux-clipboard (Android).

pub mod arboard;
pub mod provider;
pub mod stdout;
pub mod termux;

pub use arboard::ArboardClipboard;
pub use provider::{auto_detect, ClipboardError, ClipboardProvider};
pub use stdout::StdoutFallback;
pub use termux::TermuxClipboard;
