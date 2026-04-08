// Audio acquisition and chunking via ffmpeg/yt-dlp subprocesses.

pub mod acquisition;
pub mod chunker;

pub use acquisition::{transcode_to_opus, AcquisitionError, MediaAcquirer, YtDlpAcquirer};
pub use chunker::{
    adaptive_chunk, adaptive_chunk_with_policy, detect_silences, split_audio,
    split_audio_with_overlap, ChunkerError, ChunkingPolicy, CutPoint, SilenceGap,
};
