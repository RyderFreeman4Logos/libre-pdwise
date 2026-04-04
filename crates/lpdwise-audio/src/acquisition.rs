use std::path::PathBuf;

use lpdwise_core::types::{InputSource, MediaAsset};

/// Abstraction for acquiring media from various sources.
pub trait MediaAcquirer {
    /// Download or resolve the input source into a local media asset.
    fn acquire(
        &self,
        source: InputSource,
    ) -> impl std::future::Future<Output = Result<MediaAsset, AcquisitionError>>;
}

/// Acquires media via yt-dlp for URL sources.
pub struct YtDlpAcquirer {
    output_dir: PathBuf,
}

impl YtDlpAcquirer {
    pub fn new(output_dir: PathBuf) -> Self {
        Self { output_dir }
    }
}

impl MediaAcquirer for YtDlpAcquirer {
    async fn acquire(&self, _source: InputSource) -> Result<MediaAsset, AcquisitionError> {
        let _ = &self.output_dir;
        todo!("implement yt-dlp acquisition")
    }
}

/// Errors during media acquisition.
#[derive(Debug, thiserror::Error)]
pub enum AcquisitionError {
    #[error("download failed: {0}")]
    Download(String),

    #[error("file not found: {0}")]
    FileNotFound(PathBuf),

    #[error("unsupported source format")]
    UnsupportedFormat,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
