// Audio acquisition and chunking via ffmpeg/yt-dlp subprocesses.

pub mod acquisition;
pub mod chunker;

pub use acquisition::{transcode_to_opus, AcquisitionError, MediaAcquirer, YtDlpAcquirer};
pub use chunker::{
    adaptive_chunk, detect_silences, split_audio, ChunkerError, CutPoint, SilenceGap,
};
