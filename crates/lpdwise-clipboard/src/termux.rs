use anyhow::anyhow;
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
        //
        // The write is done in a dedicated thread to avoid blocking the caller
        // thread on large content that exceeds the OS pipe buffer.
        let content = content.to_owned();
        std::thread::scope(|s| {
            let mut child = Command::new("termux-clipboard-set")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()?;

            let mut stdin = child.stdin.take().ok_or_else(|| {
                ClipboardError::Unavailable(
                    anyhow!("failed to open stdin pipe for termux-clipboard-set").to_string(),
                )
            })?;

            let write_handle = s.spawn(move || stdin.write_all(content.as_bytes()));

            let output = child.wait_with_output()?;

            let write_result = write_handle.join().map_err(|_| {
                ClipboardError::Unavailable("stdin writer thread panicked".to_string())
            })?;
            write_result?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(ClipboardError::AccessDenied(format!(
                    "termux-clipboard-set failed: {stderr}"
                )));
            }

            Ok(())
        })
    }
}
