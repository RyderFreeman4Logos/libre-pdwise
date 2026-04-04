use std::path::PathBuf;

use lpdwise_core::types::ArchiveRecord;

/// Abstraction for persisting extraction results.
pub trait Archiver {
    /// Store a completed extraction record.
    fn store(&self, record: &ArchiveRecord) -> Result<(), ArchiveError>;
}

/// Archives extraction results in a local git repository.
pub struct GitArchiver {
    repo_path: PathBuf,
}

impl GitArchiver {
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
    }
}

impl Archiver for GitArchiver {
    fn store(&self, _record: &ArchiveRecord) -> Result<(), ArchiveError> {
        let _ = &self.repo_path;
        todo!("implement git-based archival")
    }
}

/// Errors from archive operations.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("git operation failed: {0}")]
    Git(String),

    #[error("serialization failed: {0}")]
    Serialization(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
