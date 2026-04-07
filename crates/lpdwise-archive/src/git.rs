use std::fs;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use git2::{Repository, Signature};
use lpdwise_core::types::{ArchiveRecord, InputSource};
use tracing::{debug, info};

/// Abstraction for persisting extraction results.
pub trait Archiver {
    /// Store a completed extraction record.
    fn store(&self, record: &ArchiveRecord) -> Result<(), ArchiveError>;
}

/// Archives extraction results in a local git repository.
///
/// Each extraction is committed as two files:
/// - `<slug>.txt`  — plain transcript text
/// - `<slug>.srt`  — SRT subtitle format
///
/// Uses fs2 flock on `archive_dir/.lock` for multi-process safety.
pub struct GitArchiver {
    repo_path: PathBuf,
}

impl GitArchiver {
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
    }

    /// Initialize or open the archive git repository.
    ///
    /// Creates the directory and `git init` if it doesn't exist.
    /// Writes a `.gitignore` to exclude media files.
    pub fn init_or_open(archive_dir: &Path) -> Result<Self, ArchiveError> {
        fs::create_dir_all(archive_dir)?;

        if archive_dir.join(".git").exists() {
            debug!(path = %archive_dir.display(), "opening existing archive repo");
            // Validate it opens correctly
            Repository::open(archive_dir).map_err(|e| ArchiveError::Git(e.to_string()))?;
        } else {
            info!(path = %archive_dir.display(), "initializing new archive repo");
            let repo =
                Repository::init(archive_dir).map_err(|e| ArchiveError::Git(e.to_string()))?;

            // Write .gitignore to exclude media files
            fs::write(
                archive_dir.join(".gitignore"),
                "# Exclude media files\n*.mp3\n*.mp4\n*.opus\n*.ogg\n*.wav\n*.flac\n*.webm\n*.mkv\n*.avi\n",
            )?;

            // Initial commit with .gitignore
            let sig = default_signature(&repo)?;
            let mut index = repo.index().map_err(|e| ArchiveError::Git(e.to_string()))?;
            index
                .add_path(Path::new(".gitignore"))
                .map_err(|e| ArchiveError::Git(e.to_string()))?;
            index
                .write()
                .map_err(|e| ArchiveError::Git(e.to_string()))?;
            let tree_oid = index
                .write_tree()
                .map_err(|e| ArchiveError::Git(e.to_string()))?;
            let tree = repo
                .find_tree(tree_oid)
                .map_err(|e| ArchiveError::Git(e.to_string()))?;
            repo.commit(
                Some("HEAD"),
                &sig,
                &sig,
                "init: archive repository",
                &tree,
                &[],
            )
            .map_err(|e| ArchiveError::Git(e.to_string()))?;
        }

        Ok(Self {
            repo_path: archive_dir.to_path_buf(),
        })
    }

    /// Acquire an exclusive file lock on `archive_dir/.lock`.
    fn lock_archive(&self) -> Result<fs::File, ArchiveError> {
        let lock_path = self.repo_path.join(".lock");
        let lock_file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)?;
        lock_file
            .lock_exclusive()
            .map_err(|e| ArchiveError::Git(format!("failed to acquire archive lock: {e}")))?;
        Ok(lock_file)
    }
}

impl Archiver for GitArchiver {
    fn store(&self, record: &ArchiveRecord) -> Result<(), ArchiveError> {
        let _lock = self.lock_archive()?;

        let repo =
            Repository::open(&self.repo_path).map_err(|e| ArchiveError::Git(e.to_string()))?;

        let slug = source_slug(&record.source);
        let txt_name = format!("{slug}.txt");
        let srt_name = format!("{slug}.srt");

        // Write transcript files
        let txt_path = self.repo_path.join(&txt_name);
        let srt_path = self.repo_path.join(&srt_name);
        fs::write(&txt_path, record.transcript.pure_text())?;
        fs::write(&srt_path, record.transcript.subtitle_text())?;

        // Stage files
        let mut index = repo.index().map_err(|e| ArchiveError::Git(e.to_string()))?;
        index
            .add_path(Path::new(&txt_name))
            .map_err(|e| ArchiveError::Git(e.to_string()))?;
        index
            .add_path(Path::new(&srt_name))
            .map_err(|e| ArchiveError::Git(e.to_string()))?;
        index
            .write()
            .map_err(|e| ArchiveError::Git(e.to_string()))?;

        let tree_oid = index
            .write_tree()
            .map_err(|e| ArchiveError::Git(e.to_string()))?;
        let tree = repo
            .find_tree(tree_oid)
            .map_err(|e| ArchiveError::Git(e.to_string()))?;

        let sig = default_signature(&repo)?;
        let source_label = match &record.source {
            InputSource::Url(u) => u.as_str(),
            InputSource::File(p) => p.to_str().unwrap_or("local-file"),
        };
        let message = format!(
            "archive: {source_label}\n\nextracted_at: {}",
            record.extracted_at
        );

        // Get parent commit (HEAD)
        let parent = repo
            .head()
            .and_then(|h| h.peel_to_commit())
            .map_err(|e| ArchiveError::Git(e.to_string()))?;

        repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[&parent])
            .map_err(|e| ArchiveError::Git(e.to_string()))?;

        info!(slug = %slug, "archived transcript to git");
        Ok(())
    }
}

/// Create a default git signature for archive commits.
fn default_signature(repo: &Repository) -> Result<Signature<'_>, ArchiveError> {
    repo.signature()
        .or_else(|_| Signature::now("lpdwise", "lpdwise@localhost"))
        .map_err(|e| ArchiveError::Git(format!("failed to create signature: {e}")))
}

/// Derive a filesystem-safe slug from the input source.
fn source_slug(source: &InputSource) -> String {
    let raw = match source {
        InputSource::Url(u) => u.clone(),
        InputSource::File(p) => p.to_string_lossy().into_owned(),
    };
    // Replace non-alphanumeric chars with hyphens, collapse multiples, trim edges
    let slug: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let collapsed = collapse_hyphens(&slug);
    let trimmed = collapsed.trim_matches('-');
    // Truncate to reasonable length
    if trimmed.len() > 120 {
        trimmed[..120].trim_end_matches('-').to_string()
    } else {
        trimmed.to_string()
    }
}

/// Collapse consecutive hyphens into a single hyphen.
fn collapse_hyphens(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_hyphen = false;
    for c in s.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push('-');
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }
    result
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use lpdwise_core::types::{Transcript, TranscriptSegment};

    use super::*;

    fn sample_record() -> ArchiveRecord {
        ArchiveRecord {
            source: InputSource::Url("https://youtube.com/watch?v=abc123".into()),
            transcript: Transcript {
                segments: vec![
                    TranscriptSegment {
                        text: "Hello world".into(),
                        start: Duration::from_secs(0),
                        end: Duration::from_secs(5),
                    },
                    TranscriptSegment {
                        text: "Second segment".into(),
                        start: Duration::from_secs(5),
                        end: Duration::from_secs(10),
                    },
                ],
            },
            extracted_at: "2025-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn test_source_slug_url() {
        let slug = source_slug(&InputSource::Url(
            "https://youtube.com/watch?v=abc123".into(),
        ));
        assert!(!slug.contains('/'));
        assert!(!slug.contains(':'));
        assert!(!slug.starts_with('-'));
        assert!(!slug.ends_with('-'));
        assert!(slug.contains("youtube"));
    }

    #[test]
    fn test_source_slug_file() {
        let slug = source_slug(&InputSource::File("/home/user/video.mp4".into()));
        assert!(!slug.contains('/'));
        assert!(slug.contains("video"));
    }

    #[test]
    fn test_source_slug_truncation() {
        let long_url = format!("https://example.com/{}", "a".repeat(200));
        let slug = source_slug(&InputSource::Url(long_url));
        assert!(slug.len() <= 120);
    }

    #[test]
    fn test_collapse_hyphens() {
        assert_eq!(collapse_hyphens("a---b--c-d"), "a-b-c-d");
        assert_eq!(collapse_hyphens("---"), "-");
        assert_eq!(collapse_hyphens("abc"), "abc");
    }

    #[test]
    fn test_init_or_open_creates_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let archive_dir = tmp.path().join("archive");

        let archiver = GitArchiver::init_or_open(&archive_dir).unwrap();
        assert!(archive_dir.join(".git").exists());
        assert!(archive_dir.join(".gitignore").exists());

        // Opening again should succeed
        let _archiver2 = GitArchiver::init_or_open(&archive_dir).unwrap();
        drop(archiver);
    }

    #[test]
    fn test_store_creates_files_and_commits() {
        let tmp = tempfile::tempdir().unwrap();
        let archive_dir = tmp.path().join("archive");

        let archiver = GitArchiver::init_or_open(&archive_dir).unwrap();
        let record = sample_record();
        archiver.store(&record).unwrap();

        // Check files exist
        let slug = source_slug(&record.source);
        assert!(archive_dir.join(format!("{slug}.txt")).exists());
        assert!(archive_dir.join(format!("{slug}.srt")).exists());

        // Verify git log has 2 commits (init + archive)
        let repo = Repository::open(&archive_dir).unwrap();
        let mut revwalk = repo.revwalk().unwrap();
        revwalk.push_head().unwrap();
        assert_eq!(revwalk.count(), 2);
    }

    #[test]
    fn test_store_multiple_records() {
        let tmp = tempfile::tempdir().unwrap();
        let archive_dir = tmp.path().join("archive");

        let archiver = GitArchiver::init_or_open(&archive_dir).unwrap();

        let mut record1 = sample_record();
        record1.source = InputSource::Url("https://example.com/video1".into());
        archiver.store(&record1).unwrap();

        let mut record2 = sample_record();
        record2.source = InputSource::Url("https://example.com/video2".into());
        archiver.store(&record2).unwrap();

        let repo = Repository::open(&archive_dir).unwrap();
        let mut revwalk = repo.revwalk().unwrap();
        revwalk.push_head().unwrap();
        assert_eq!(revwalk.count(), 3); // init + 2 archives
    }
}
