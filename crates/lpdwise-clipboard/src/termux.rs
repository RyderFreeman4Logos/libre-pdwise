use std::io::Write;
use std::process::{Command, Stdio};

use crate::provider::{ClipboardError, ClipboardProvider};

/// Clipboard provider for Termux (Android) via `termux-clipboard-{get,set}`.
#[derive(Default)]
pub struct TermuxClipboard;

impl TermuxClipboard {
    pub fn new() -> Self {
        Self
    }
}

impl ClipboardProvider for TermuxClipboard {
    fn get_content(&self) -> Result<String, ClipboardError> {
        let output = Command::new("termux-clipboard-get")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ClipboardError::AccessDenied(format!(
                "termux-clipboard-get failed: {stderr}"
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn set_content(&self, content: &str) -> Result<(), ClipboardError> {
        // Write content via stdin pipe instead of command-line argument to
        // prevent sensitive data from appearing in process listings.
        let mut child = Command::new("termux-clipboard-set")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;

        if let Some(ref mut stdin) = child.stdin {
            stdin.write_all(content.as_bytes())?;
        }
        // Close stdin to signal EOF
        drop(child.stdin.take());

        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ClipboardError::AccessDenied(format!(
                "termux-clipboard-set failed: {stderr}"
            )));
        }

        Ok(())
    }
}
