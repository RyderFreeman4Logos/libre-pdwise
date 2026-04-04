use std::cell::RefCell;

use crate::provider::{ClipboardError, ClipboardProvider};

/// Desktop clipboard via the `arboard` crate.
///
/// Uses `RefCell` internally because arboard requires `&mut self` but our
/// `ClipboardProvider` trait uses `&self` for ergonomic dynamic dispatch.
pub struct ArboardClipboard {
    inner: RefCell<arboard::Clipboard>,
}

impl ArboardClipboard {
    pub fn new() -> Result<Self, ClipboardError> {
        let inner =
            arboard::Clipboard::new().map_err(|e| ClipboardError::Unavailable(e.to_string()))?;
        Ok(Self {
            inner: RefCell::new(inner),
        })
    }
}

impl ClipboardProvider for ArboardClipboard {
    fn get_content(&self) -> Result<String, ClipboardError> {
        self.inner
            .borrow_mut()
            .get_text()
            .map_err(|e| ClipboardError::AccessDenied(e.to_string()))
    }

    fn set_content(&self, content: &str) -> Result<(), ClipboardError> {
        self.inner
            .borrow_mut()
            .set_text(content)
            .map_err(|e| ClipboardError::AccessDenied(e.to_string()))
    }
}
