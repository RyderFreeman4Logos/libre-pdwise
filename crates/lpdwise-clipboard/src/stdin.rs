use crate::provider::{ClipboardError, ClipboardProvider};

/// Universal fallback provider that reads from stdin and writes to stdout.
pub struct StdinProvider;

impl StdinProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StdinProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardProvider for StdinProvider {
    fn get_content(&self) -> Result<String, ClipboardError> {
        eprintln!("No clipboard backend available. Please paste your input and press Enter:");
        let mut buffer = String::new();
        std::io::stdin().read_line(&mut buffer)?;
        Ok(buffer.trim_end_matches(&['\r', '\n'][..]).to_string())
    }

    fn set_content(&self, content: &str) -> Result<(), ClipboardError> {
        println!("{content}");
        Ok(())
    }
}
