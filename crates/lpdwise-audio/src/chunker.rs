use std::path::Path;
use std::time::Duration;

use regex::Regex;
use tracing::{debug, info, warn};

use lpdwise_core::types::AudioChunk;
use lpdwise_process::runner::{CommandRunner, ProcessRunner};

/// Default maximum chunk size in bytes for legacy size-only chunking.
const DEFAULT_MAX_CHUNK_BYTES: u64 = 25 * 1024 * 1024;
/// Conservative Groq upload budget kept below the provider hard cap.
const GROQ_SAFE_MAX_CHUNK_BYTES: u64 = 20 * 1024 * 1024;
const GROQ_MAX_CHUNK_DURATION_MS: u64 = 10 * 60 * 1000;
const GROQ_TARGET_CHUNK_DURATION_MS: u64 = 5 * 60 * 1000;
const GROQ_MIN_CHUNK_DURATION_MS: u64 = 90 * 1000;
const GROQ_OVERLAP_MS: u64 = 10 * 1000;

/// A detected gap of silence in the audio stream.
#[derive(Debug, Clone, PartialEq)]
pub struct SilenceGap {
    pub start_ms: u64,
    pub end_ms: u64,
    pub duration_ms: u64,
}

/// A point at which to cut the audio, positioned at a silence midpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct CutPoint {
    /// Millisecond offset where the cut should be made.
    pub offset_ms: u64,
}

/// Chunking knobs for engines that need tighter context control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkingPolicy {
    pub max_chunk_bytes: u64,
    pub max_chunk_duration_ms: u64,
    pub target_chunk_duration_ms: u64,
    pub min_chunk_duration_ms: u64,
    pub overlap_ms: u64,
}

impl ChunkingPolicy {
    /// Conservative Groq policy:
    /// - 20 MiB safe upload budget
    /// - 5 min target chunks
    /// - 10 min hard ceiling
    /// - 10 s backward overlap
    pub const fn groq_whisper() -> Self {
        Self {
            max_chunk_bytes: GROQ_SAFE_MAX_CHUNK_BYTES,
            max_chunk_duration_ms: GROQ_MAX_CHUNK_DURATION_MS,
            target_chunk_duration_ms: GROQ_TARGET_CHUNK_DURATION_MS,
            min_chunk_duration_ms: GROQ_MIN_CHUNK_DURATION_MS,
            overlap_ms: GROQ_OVERLAP_MS,
        }
    }
}

/// Errors from audio chunking.
#[derive(Debug, thiserror::Error)]
pub enum ChunkerError {
    #[error("ffmpeg silence detection failed: {0}")]
    SilenceDetection(String),

    #[error("ffmpeg split failed: {0}")]
    SplitFailed(String),

    #[error("no audio data in asset")]
    EmptyAudio,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// 3-1: silencedetect parser
// ---------------------------------------------------------------------------

/// Run ffmpeg silencedetect and parse the output into silence gaps.
pub async fn detect_silences(
    audio_path: &Path,
    runner: &CommandRunner,
) -> Result<Vec<SilenceGap>, ChunkerError> {
    let path_str = audio_path.to_string_lossy();

    info!(path = %path_str, "detecting silence gaps via ffmpeg");

    let output = runner
        .run_streaming(
            "ffmpeg",
            &[
                "-i",
                &path_str,
                "-af",
                "silencedetect=noise=-30dB:d=0.3",
                "-f",
                "null",
                "-",
            ],
            None,
        )
        .await;

    // ffmpeg writes silencedetect output to stderr and exits 0.
    // On non-zero exit we still try to parse whatever stderr we got.
    let stderr = match output {
        Ok(o) => o.stderr,
        Err(lpdwise_process::runner::ProcessError::NonZeroExit { output: o, .. }) => o.stderr,
        Err(e) => return Err(ChunkerError::SilenceDetection(e.to_string())),
    };

    Ok(parse_silencedetect_output(&stderr))
}

/// Parse ffmpeg silencedetect stderr lines into `SilenceGap` entries.
///
/// Expected lines look like:
///   [silencedetect @ ...] silence_start: 1.234
///   [silencedetect @ ...] silence_end: 2.567 | silence_duration: 1.333
fn parse_silencedetect_output(stderr: &str) -> Vec<SilenceGap> {
    let start_re = Regex::new(r"silence_start:\s*([\d.]+)").expect("valid regex");
    let end_re = Regex::new(r"silence_end:\s*([\d.]+)\s*\|\s*silence_duration:\s*([\d.]+)")
        .expect("valid regex");

    let mut pending_start: Option<u64> = None;
    let mut gaps = Vec::new();

    for line in stderr.lines() {
        if let Some(caps) = start_re.captures(line) {
            let secs: f64 = caps[1].parse().unwrap_or(0.0);
            pending_start = Some(seconds_to_ms(secs));
        }

        if let Some(caps) = end_re.captures(line) {
            let end_secs: f64 = caps[1].parse().unwrap_or(0.0);
            let dur_secs: f64 = caps[2].parse().unwrap_or(0.0);
            let end_ms = seconds_to_ms(end_secs);
            let duration_ms = seconds_to_ms(dur_secs);

            let start_ms = pending_start.unwrap_or_else(|| end_ms.saturating_sub(duration_ms));

            gaps.push(SilenceGap {
                start_ms,
                end_ms,
                duration_ms,
            });
            pending_start = None;
        }
    }

    debug!(count = gaps.len(), "parsed silence gaps");
    gaps
}

// ---------------------------------------------------------------------------
// 3-2: adaptive minimum-cut algorithm
// ---------------------------------------------------------------------------

/// Compute the minimum set of cut points so every resulting chunk is
/// smaller than `max_chunk_bytes`.
///
/// Algorithm:
/// 1. If `total_size_bytes <= max_chunk_bytes`, return empty (no cuts needed).
/// 2. Compute minimum number of pieces: `ceil(total / max)`.
/// 3. Number of cuts needed: `pieces - 1`.
/// 4. Sort silence gaps by duration descending, pick the top N cuts.
/// 5. For each selected gap, cut at its midpoint.
/// 6. If fewer silence gaps than needed, fall back to fixed-duration cuts.
pub fn adaptive_chunk(
    silences: &[SilenceGap],
    total_duration_ms: u64,
    total_size_bytes: u64,
    max_chunk_bytes: u64,
) -> Vec<CutPoint> {
    let max_bytes = if max_chunk_bytes == 0 {
        DEFAULT_MAX_CHUNK_BYTES
    } else {
        max_chunk_bytes
    };

    if total_size_bytes <= max_bytes || total_duration_ms == 0 {
        return Vec::new();
    }

    let pieces = div_ceil(total_size_bytes, max_bytes);
    let cuts_needed = (pieces - 1) as usize;

    if cuts_needed == 0 {
        return Vec::new();
    }

    // Sort by duration descending to prefer cutting at longest silences.
    let mut ranked: Vec<&SilenceGap> = silences.iter().collect();
    ranked.sort_by(|a, b| b.duration_ms.cmp(&a.duration_ms));

    let mut cut_offsets: Vec<u64> = Vec::with_capacity(cuts_needed);

    for gap in ranked.iter().take(cuts_needed) {
        // Cut at the midpoint of the silence gap.
        let midpoint = gap.start_ms + gap.duration_ms / 2;
        cut_offsets.push(midpoint);
    }

    // Fall back to fixed-duration cuts if not enough silence gaps.
    if cut_offsets.len() < cuts_needed {
        let remaining = cuts_needed - cut_offsets.len();
        let interval = total_duration_ms / (remaining as u64 + 1);
        for i in 1..=remaining {
            let offset = interval * i as u64;
            // Avoid duplicates with existing cuts.
            if !cut_offsets.contains(&offset) {
                cut_offsets.push(offset);
            }
        }
    }

    // Sort chronologically.
    cut_offsets.sort_unstable();

    // Deduplicate and filter out 0 / total_duration boundary values.
    cut_offsets.dedup();
    cut_offsets.retain(|&ms| ms > 0 && ms < total_duration_ms);

    info!(
        cuts = cut_offsets.len(),
        pieces, total_size_bytes, max_bytes, "computed adaptive chunk cut points"
    );

    cut_offsets
        .into_iter()
        .map(|ms| CutPoint { offset_ms: ms })
        .collect()
}

/// Compute cut points with both size and time constraints.
///
/// This is intended for cloud ASR backends where oversize chunks hurt both
/// request success rate and recognition quality.
pub fn adaptive_chunk_with_policy(
    silences: &[SilenceGap],
    total_duration_ms: u64,
    total_size_bytes: u64,
    policy: ChunkingPolicy,
) -> Vec<CutPoint> {
    if total_duration_ms == 0 {
        return Vec::new();
    }

    let effective_max_duration_ms =
        effective_max_chunk_duration_ms(total_duration_ms, total_size_bytes, policy);
    if effective_max_duration_ms == 0 {
        return Vec::new();
    }

    if total_duration_ms <= effective_max_duration_ms
        && (total_size_bytes == 0 || total_size_bytes <= policy.max_chunk_bytes)
    {
        return Vec::new();
    }

    let target_chunk_duration_ms = policy
        .target_chunk_duration_ms
        .min(effective_max_duration_ms);
    let min_chunk_duration_ms = policy
        .min_chunk_duration_ms
        .min(target_chunk_duration_ms)
        .min(effective_max_duration_ms);

    let mut cuts = Vec::new();
    let mut pos_ms = 0;

    while pos_ms + min_chunk_duration_ms < total_duration_ms {
        let remaining_ms = total_duration_ms.saturating_sub(pos_ms);
        if remaining_ms <= effective_max_duration_ms {
            break;
        }

        let window_end_ms = (pos_ms + effective_max_duration_ms).min(total_duration_ms);
        let min_cut_ms = pos_ms + min_chunk_duration_ms;

        let candidates = silence_candidates(silences, min_cut_ms, window_end_ms);
        let fallback_candidates = silence_candidates(silences, pos_ms, window_end_ms);

        let best = candidates
            .iter()
            .max_by(|left, right| {
                score_gap(left, pos_ms, target_chunk_duration_ms).total_cmp(&score_gap(
                    right,
                    pos_ms,
                    target_chunk_duration_ms,
                ))
            })
            .or_else(|| {
                fallback_candidates.iter().max_by(|left, right| {
                    score_gap(left, pos_ms, target_chunk_duration_ms).total_cmp(&score_gap(
                        right,
                        pos_ms,
                        target_chunk_duration_ms,
                    ))
                })
            });

        let cut_ms = match best {
            Some(gap) => gap.start_ms + gap.duration_ms / 2,
            None => window_end_ms,
        };

        let cut_ms = cut_ms.clamp(
            pos_ms.saturating_add(1),
            total_duration_ms.saturating_sub(1),
        );
        cuts.push(CutPoint { offset_ms: cut_ms });
        pos_ms = cut_ms;
    }

    cuts.dedup_by_key(|cut| cut.offset_ms);
    cuts.retain(|cut| cut.offset_ms > 0 && cut.offset_ms < total_duration_ms);

    info!(
        cuts = cuts.len(),
        total_duration_ms,
        total_size_bytes,
        effective_max_duration_ms,
        "computed policy-based chunk cut points"
    );

    cuts
}

// ---------------------------------------------------------------------------
// 3-3: ffmpeg split execution
// ---------------------------------------------------------------------------

/// Split an audio file at the given cut points, writing chunks to `output_dir`.
///
/// Returns an `AudioChunk` for each resulting segment (including the implicit
/// first and last pieces around the cut points).
pub async fn split_audio(
    audio_path: &Path,
    cut_points: &[CutPoint],
    output_dir: &Path,
    runner: &CommandRunner,
    total_duration_ms: u64,
) -> Result<Vec<AudioChunk>, ChunkerError> {
    split_audio_with_overlap(
        audio_path,
        cut_points,
        output_dir,
        runner,
        total_duration_ms,
        0,
    )
    .await
}

/// Split an audio file at the given cut points, adding backward overlap to
/// chunks after the first.
pub async fn split_audio_with_overlap(
    audio_path: &Path,
    cut_points: &[CutPoint],
    output_dir: &Path,
    runner: &CommandRunner,
    total_duration_ms: u64,
    overlap_ms: u64,
) -> Result<Vec<AudioChunk>, ChunkerError> {
    if total_duration_ms == 0 {
        return Err(ChunkerError::EmptyAudio);
    }

    // No cuts needed — the whole file is one chunk.
    if cut_points.is_empty() {
        return Ok(vec![AudioChunk {
            path: audio_path.to_path_buf(),
            index: 0,
            audio_start: Duration::ZERO,
            start: Duration::ZERO,
            end: Duration::from_millis(total_duration_ms),
        }]);
    }

    let chunk_plan = plan_split_chunks(
        audio_path,
        cut_points,
        output_dir,
        total_duration_ms,
        overlap_ms,
    );
    let mut chunks = Vec::with_capacity(chunk_plan.len());

    for chunk in chunk_plan {
        let input_str = audio_path.to_string_lossy();
        let output_str = chunk.path.to_string_lossy();

        let start_secs = format!("{:.3}", chunk.audio_start.as_secs_f64());
        let end_secs = format!("{:.3}", chunk.end.as_secs_f64());

        info!(
            index = chunk.index,
            audio_start_ms = chunk.audio_start.as_millis(),
            start_ms = chunk.start.as_millis(),
            end_ms = chunk.end.as_millis(),
            output = %output_str,
            "splitting audio chunk"
        );

        runner
            .run_streaming(
                "ffmpeg",
                &[
                    "-i",
                    &input_str,
                    "-ss",
                    &start_secs,
                    "-to",
                    &end_secs,
                    "-c",
                    "copy",
                    "-y",
                    &output_str,
                ],
                None,
            )
            .await
            .map_err(|e| ChunkerError::SplitFailed(format!("chunk {}: {e}", chunk.index)))?;

        if !chunk.path.exists() {
            warn!(path = %output_str, "expected chunk file not found after ffmpeg");
            return Err(ChunkerError::SplitFailed(format!(
                "output file missing: {output_str}"
            )));
        }

        chunks.push(chunk);
    }

    Ok(chunks)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn seconds_to_ms(secs: f64) -> u64 {
    (secs * 1000.0).round() as u64
}

fn effective_max_chunk_duration_ms(
    total_duration_ms: u64,
    total_size_bytes: u64,
    policy: ChunkingPolicy,
) -> u64 {
    let size_bound_ms = if total_size_bytes == 0 || total_size_bytes <= policy.max_chunk_bytes {
        total_duration_ms
    } else {
        let scaled = (u128::from(total_duration_ms) * u128::from(policy.max_chunk_bytes))
            / u128::from(total_size_bytes);
        scaled.max(1).min(u128::from(u64::MAX)) as u64
    };

    policy
        .max_chunk_duration_ms
        .min(size_bound_ms)
        .min(total_duration_ms)
}

fn silence_candidates(
    silences: &[SilenceGap],
    window_start_ms: u64,
    window_end_ms: u64,
) -> Vec<&SilenceGap> {
    silences
        .iter()
        .filter(|gap| gap.start_ms >= window_start_ms && gap.end_ms <= window_end_ms)
        .collect()
}

fn score_gap(gap: &SilenceGap, pos_ms: u64, target_chunk_duration_ms: u64) -> f64 {
    let midpoint_ms = gap.start_ms as f64 + gap.duration_ms as f64 / 2.0;
    let chunk_len_ms = midpoint_ms - pos_ms as f64;
    let mut quality = (1.0 + gap.duration_ms as f64 / 1000.0).ln();
    if gap.duration_ms >= 2_000 {
        quality *= 1.5;
    }

    let sigma = (target_chunk_duration_ms as f64 * 0.4).max(1.0);
    let distance = (chunk_len_ms - target_chunk_duration_ms as f64).abs();
    let proximity = (-0.5 * (distance / sigma).powi(2)).exp();

    quality * proximity
}

fn plan_split_chunks(
    audio_path: &Path,
    cut_points: &[CutPoint],
    output_dir: &Path,
    total_duration_ms: u64,
    overlap_ms: u64,
) -> Vec<AudioChunk> {
    let stem = audio_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("chunk");
    let ext = audio_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("opus");

    let mut boundaries: Vec<u64> = Vec::with_capacity(cut_points.len() + 2);
    boundaries.push(0);
    for cp in cut_points {
        boundaries.push(cp.offset_ms);
    }
    boundaries.push(total_duration_ms);

    let mut chunks = Vec::with_capacity(boundaries.len().saturating_sub(1));

    for (index, window) in boundaries.windows(2).enumerate() {
        let start_ms = window[0];
        let end_ms = window[1];
        let chunk_duration_ms = end_ms.saturating_sub(start_ms);
        let backward_overlap_ms = if index == 0 || overlap_ms == 0 {
            0
        } else {
            overlap_ms.min(chunk_duration_ms / 3).min(start_ms)
        };
        let audio_start_ms = start_ms.saturating_sub(backward_overlap_ms);

        chunks.push(AudioChunk {
            path: output_dir.join(format!("{stem}_{index:04}.{ext}")),
            index,
            audio_start: Duration::from_millis(audio_start_ms),
            start: Duration::from_millis(start_ms),
            end: Duration::from_millis(end_ms),
        });
    }

    chunks
}

/// Integer ceiling division.
fn div_ceil(a: u64, b: u64) -> u64 {
    a.div_ceil(b)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_silencedetect_output ---

    #[test]
    fn test_parse_silencedetect_basic() {
        let stderr = "\
[silencedetect @ 0x1234] silence_start: 1.5
[silencedetect @ 0x1234] silence_end: 2.8 | silence_duration: 1.3
[silencedetect @ 0x1234] silence_start: 10.0
[silencedetect @ 0x1234] silence_end: 11.5 | silence_duration: 1.5
";
        let gaps = parse_silencedetect_output(stderr);
        assert_eq!(gaps.len(), 2);

        assert_eq!(gaps[0].start_ms, 1500);
        assert_eq!(gaps[0].end_ms, 2800);
        assert_eq!(gaps[0].duration_ms, 1300);

        assert_eq!(gaps[1].start_ms, 10000);
        assert_eq!(gaps[1].end_ms, 11500);
        assert_eq!(gaps[1].duration_ms, 1500);
    }

    #[test]
    fn test_parse_silencedetect_empty() {
        let gaps = parse_silencedetect_output("");
        assert!(gaps.is_empty());
    }

    #[test]
    fn test_parse_silencedetect_no_start_line() {
        // Some ffmpeg versions may only emit end lines.
        let stderr = "[silencedetect @ 0x1] silence_end: 5.0 | silence_duration: 2.0\n";
        let gaps = parse_silencedetect_output(stderr);
        assert_eq!(gaps.len(), 1);
        // Inferred start: end - duration = 5000 - 2000 = 3000.
        assert_eq!(gaps[0].start_ms, 3000);
        assert_eq!(gaps[0].end_ms, 5000);
    }

    // --- adaptive_chunk ---

    #[test]
    fn test_adaptive_no_cut_when_small() {
        let cuts = adaptive_chunk(&[], 60_000, 10_000_000, DEFAULT_MAX_CHUNK_BYTES);
        assert!(cuts.is_empty());
    }

    #[test]
    fn test_adaptive_single_cut() {
        // 50 MB total, 25 MB max → need 1 cut (2 pieces).
        let silences = vec![
            SilenceGap {
                start_ms: 29_000,
                end_ms: 31_000,
                duration_ms: 2000,
            },
            SilenceGap {
                start_ms: 10_000,
                end_ms: 10_500,
                duration_ms: 500,
            },
        ];

        let cuts = adaptive_chunk(&silences, 60_000, 50 * 1024 * 1024, DEFAULT_MAX_CHUNK_BYTES);
        assert_eq!(cuts.len(), 1);
        // Should pick the longest silence (2000ms), midpoint = 29000 + 1000 = 30000.
        assert_eq!(cuts[0].offset_ms, 30_000);
    }

    #[test]
    fn test_adaptive_fallback_fixed_duration() {
        // 75 MB → 3 pieces → 2 cuts needed, but no silence gaps.
        let cuts = adaptive_chunk(&[], 120_000, 75 * 1024 * 1024, DEFAULT_MAX_CHUNK_BYTES);
        assert_eq!(cuts.len(), 2);
        // Fixed intervals: 120000 / 3 = 40000 → cuts at 40000, 80000.
        assert_eq!(cuts[0].offset_ms, 40_000);
        assert_eq!(cuts[1].offset_ms, 80_000);
    }

    #[test]
    fn test_adaptive_many_cuts() {
        // 150 MB → 6 pieces → 5 cuts.
        let silences: Vec<SilenceGap> = (0..10)
            .map(|i| SilenceGap {
                start_ms: i * 10_000,
                end_ms: i * 10_000 + 1000,
                duration_ms: 1000,
            })
            .collect();

        let cuts = adaptive_chunk(
            &silences,
            100_000,
            150 * 1024 * 1024,
            DEFAULT_MAX_CHUNK_BYTES,
        );
        assert_eq!(cuts.len(), 5);
        // All cut points should be within valid range.
        for cp in &cuts {
            assert!(cp.offset_ms > 0);
            assert!(cp.offset_ms < 100_000);
        }
    }

    #[test]
    fn test_policy_chunking_limits_duration_even_when_file_is_small() {
        let silences = vec![
            SilenceGap {
                start_ms: 295_000,
                end_ms: 305_000,
                duration_ms: 10_000,
            },
            SilenceGap {
                start_ms: 895_000,
                end_ms: 905_000,
                duration_ms: 10_000,
            },
        ];

        let cuts = adaptive_chunk_with_policy(
            &silences,
            1_200_000,
            5 * 1024 * 1024,
            ChunkingPolicy::groq_whisper(),
        );

        assert_eq!(cuts.len(), 2);
        assert_eq!(cuts[0].offset_ms, 300_000);
        assert_eq!(cuts[1].offset_ms, 900_000);
    }

    #[test]
    fn test_policy_chunking_uses_size_bound_when_bitrate_is_high() {
        let silences = vec![
            SilenceGap {
                start_ms: 110_000,
                end_ms: 130_000,
                duration_ms: 20_000,
            },
            SilenceGap {
                start_ms: 260_000,
                end_ms: 270_000,
                duration_ms: 10_000,
            },
        ];

        let cuts = adaptive_chunk_with_policy(
            &silences,
            600_000,
            80 * 1024 * 1024,
            ChunkingPolicy::groq_whisper(),
        );

        assert!(!cuts.is_empty());
        assert_eq!(cuts[0].offset_ms, 120_000);
    }

    // --- split_audio returns single chunk when no cuts ---

    #[tokio::test]
    async fn test_split_audio_no_cuts_returns_original() {
        let tmp = tempfile::tempdir().unwrap();
        let fake_audio = tmp.path().join("test.opus");
        std::fs::write(&fake_audio, b"fake audio").unwrap();

        let runner = CommandRunner::with_default_timeout();
        let result = split_audio(&fake_audio, &[], tmp.path(), &runner, 60_000)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].index, 0);
        assert_eq!(result[0].audio_start, Duration::ZERO);
        assert_eq!(result[0].start, Duration::ZERO);
        assert_eq!(result[0].end, Duration::from_millis(60_000));
        assert_eq!(result[0].path, fake_audio);
    }

    #[tokio::test]
    async fn test_split_audio_empty_duration_error() {
        let tmp = tempfile::tempdir().unwrap();
        let fake_audio = tmp.path().join("empty.opus");
        std::fs::write(&fake_audio, b"").unwrap();

        let runner = CommandRunner::with_default_timeout();
        let result = split_audio(&fake_audio, &[], tmp.path(), &runner, 0).await;

        assert!(result.is_err());
    }

    #[test]
    fn test_plan_split_chunks_applies_backward_overlap() {
        let audio_path = Path::new("/tmp/source.opus");
        let output_dir = Path::new("/tmp/chunks");
        let cut_points = vec![CutPoint { offset_ms: 120_000 }];

        let chunks = plan_split_chunks(audio_path, &cut_points, output_dir, 300_000, 10_000);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].audio_start, Duration::ZERO);
        assert_eq!(chunks[0].start, Duration::ZERO);
        assert_eq!(chunks[0].end, Duration::from_millis(120_000));

        assert_eq!(chunks[1].audio_start, Duration::from_millis(110_000));
        assert_eq!(chunks[1].start, Duration::from_millis(120_000));
        assert_eq!(chunks[1].end, Duration::from_millis(300_000));
    }

    // --- helpers ---

    #[test]
    fn test_seconds_to_ms() {
        assert_eq!(seconds_to_ms(1.5), 1500);
        assert_eq!(seconds_to_ms(0.0), 0);
        assert_eq!(seconds_to_ms(123.456), 123456);
    }

    #[test]
    fn test_div_ceil() {
        assert_eq!(div_ceil(10, 3), 4);
        assert_eq!(div_ceil(9, 3), 3);
        assert_eq!(div_ceil(1, 1), 1);
        assert_eq!(div_ceil(25 * 1024 * 1024, 25 * 1024 * 1024), 1);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    const MB: u64 = 1024 * 1024;

    /// Generate a valid silence gap within `[0, max_ms)`.
    fn arb_silence_gap(max_ms: u64) -> impl Strategy<Value = SilenceGap> {
        (0..max_ms, 100..=5000u64).prop_map(move |(start, dur)| {
            let end = (start + dur).min(max_ms);
            let actual_dur = end.saturating_sub(start);
            SilenceGap {
                start_ms: start,
                end_ms: end,
                duration_ms: actual_dur,
            }
        })
    }

    proptest! {
        /// No cut when total size fits in one chunk.
        #[test]
        fn prop_no_cut_when_small(
            duration_ms in 1_000u64..600_000,
            size_bytes in 1u64..DEFAULT_MAX_CHUNK_BYTES,
        ) {
            let cuts = adaptive_chunk(&[], duration_ms, size_bytes, DEFAULT_MAX_CHUNK_BYTES);
            prop_assert!(cuts.is_empty(), "should not cut when size fits");
        }

        /// All cut points are strictly within `(0, total_duration_ms)`.
        #[test]
        fn prop_cuts_within_bounds(
            duration_ms in 10_000u64..600_000,
            size_bytes in (DEFAULT_MAX_CHUNK_BYTES + 1)..500 * MB,
        ) {
            let silences: Vec<SilenceGap> = (0..5)
                .map(|i| {
                    let s = duration_ms / 6 * (i + 1);
                    SilenceGap { start_ms: s, end_ms: s + 500, duration_ms: 500 }
                })
                .collect();

            let cuts = adaptive_chunk(&silences, duration_ms, size_bytes, DEFAULT_MAX_CHUNK_BYTES);
            for cp in &cuts {
                prop_assert!(cp.offset_ms > 0, "cut must be > 0");
                prop_assert!(
                    cp.offset_ms < duration_ms,
                    "cut {} must be < duration {}",
                    cp.offset_ms,
                    duration_ms,
                );
            }
        }

        /// Number of resulting pieces >= ceil(size / max).
        #[test]
        fn prop_enough_pieces(
            duration_ms in 10_000u64..600_000,
            size_bytes in (DEFAULT_MAX_CHUNK_BYTES + 1)..300 * MB,
        ) {
            let cuts = adaptive_chunk(&[], duration_ms, size_bytes, DEFAULT_MAX_CHUNK_BYTES);
            let pieces = cuts.len() as u64 + 1;
            let min_pieces = div_ceil(size_bytes, DEFAULT_MAX_CHUNK_BYTES);
            prop_assert!(
                pieces >= min_pieces,
                "got {} pieces, need at least {}",
                pieces,
                min_pieces,
            );
        }

        /// Cut points are sorted and unique.
        #[test]
        fn prop_cuts_sorted_unique(
            duration_ms in 30_000u64..600_000,
            size_bytes in (DEFAULT_MAX_CHUNK_BYTES + 1)..300 * MB,
            silences in proptest::collection::vec(arb_silence_gap(600_000), 0..20),
        ) {
            let cuts = adaptive_chunk(&silences, duration_ms, size_bytes, DEFAULT_MAX_CHUNK_BYTES);
            for window in cuts.windows(2) {
                prop_assert!(
                    window[0].offset_ms < window[1].offset_ms,
                    "cuts must be strictly increasing: {} >= {}",
                    window[0].offset_ms,
                    window[1].offset_ms,
                );
            }
        }

        /// Fallback to fixed-duration cuts when no silences are available.
        #[test]
        fn prop_fallback_no_silence(
            duration_ms in 10_000u64..600_000,
            num_pieces in 2u64..10,
        ) {
            let size_bytes = num_pieces * DEFAULT_MAX_CHUNK_BYTES;
            let cuts = adaptive_chunk(&[], duration_ms, size_bytes, DEFAULT_MAX_CHUNK_BYTES);
            // We need at least `num_pieces - 1` cuts.
            prop_assert!(
                cuts.len() as u64 >= num_pieces - 1,
                "need {} cuts for {} pieces, got {}",
                num_pieces - 1,
                num_pieces,
                cuts.len(),
            );
        }
    }
}
