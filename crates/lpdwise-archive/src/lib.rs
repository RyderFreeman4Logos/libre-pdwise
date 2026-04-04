// Local git2-based archive for transcription results.

pub mod git;

pub use git::{ArchiveError, Archiver, GitArchiver};
