use std::path::{Path, PathBuf};
use std::time::Duration;

use regex::Regex;
use tracing::{debug, info};

use lpdwise_core::types::{InputSource, MediaAsset};
use lpdwise_process::runner::{CommandRunner, ProcessError, ProcessRunner};

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
    runner: CommandRunner,
}

impl YtDlpAcquirer {
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            output_dir,
            runner: CommandRunner::with_default_timeout(),
        }
    }

    pub fn with_runner(output_dir: PathBuf, runner: CommandRunner) -> Self {
        Self { output_dir, runner }
    }
}

impl MediaAcquirer for YtDlpAcquirer {
    async fn acquire(&self, source: InputSource) -> Result<MediaAsset, AcquisitionError> {
        let url = match &source {
            InputSource::Url(url) => url.clone(),
            InputSource::File(_) => return Err(AcquisitionError::UnsupportedFormat),
        };

        validate_url(&url)?;

        let output_filename = sanitize_filename(&url);
        let output_path = self.output_dir.join(&output_filename);
        let output_str = output_path.to_string_lossy().to_string();

        info!(url = %url, output = %output_str, "acquiring audio via yt-dlp");

        self.runner
            .run_streaming(
                "yt-dlp",
                &[
                    "-x",
                    "--audio-format",
                    "opus",
                    "--audio-quality",
                    "0",
                    "--newline",
                    "-o",
                    &output_str,
                    &url,
                ],
                None,
            )
            .await
            .map_err(|e| AcquisitionError::Download(format!("yt-dlp failed: {e}")))?;

        // yt-dlp may append .opus to the output path
        let actual_path = find_output_file(&output_path)?;

        let (duration, size_bytes) = probe_media(&actual_path, &self.runner).await?;

        Ok(MediaAsset {
            source,
            path: actual_path,
            duration,
            size_bytes: Some(size_bytes),
        })
    }
}

/// Transcode audio to Opus format via ffmpeg.
///
/// Skips transcoding if the input is already Opus-encoded.
pub async fn transcode_to_opus(
    input: &Path,
    output: &Path,
    runner: &CommandRunner,
) -> Result<MediaAsset, AcquisitionError> {
    // Skip transcoding if already opus
    if is_opus(input, runner).await {
        debug!(path = %input.display(), "input already opus, skipping transcode");
        let (duration, size_bytes) = probe_media(input, runner).await?;
        return Ok(MediaAsset {
            source: InputSource::File(input.to_path_buf()),
            path: input.to_path_buf(),
            duration,
            size_bytes: Some(size_bytes),
        });
    }

    info!(
        input = %input.display(),
        output = %output.display(),
        "transcoding to opus via ffmpeg"
    );

    let input_str = input.to_string_lossy();
    let output_str = output.to_string_lossy();

    runner
        .run_streaming(
            "ffmpeg",
            &[
                "-i",
                &input_str,
                "-c:a",
                "libopus",
                "-b:a",
                "128k",
                "-y",
                &output_str,
            ],
            None,
        )
        .await
        .map_err(|e| AcquisitionError::Transcode(format!("ffmpeg failed: {e}")))?;

    if !output.exists() {
        return Err(AcquisitionError::FileNotFound(output.to_path_buf()));
    }

    let (duration, size_bytes) = probe_media(output, runner).await?;

    Ok(MediaAsset {
        source: InputSource::File(input.to_path_buf()),
        path: output.to_path_buf(),
        duration,
        size_bytes: Some(size_bytes),
    })
}

/// Validate that a URL matches the HTTP(S) whitelist pattern.
fn validate_url(url: &str) -> Result<(), AcquisitionError> {
    let re = Regex::new(r"^https?://[^\s]+$").expect("valid regex");
    if re.is_match(url) {
        Ok(())
    } else {
        Err(AcquisitionError::Download(format!(
            "invalid URL (must be http/https): {url}"
        )))
    }
}

/// Sanitize a URL into a safe filename, keeping only alphanumeric chars and .-_
fn sanitize_filename(url: &str) -> String {
    let base: String = url
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();

    // Prevent path traversal by stripping leading dots and slashes
    let trimmed = base.trim_start_matches(['.', '_']);
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Find the actual output file, checking common extensions yt-dlp may append.
fn find_output_file(base_path: &Path) -> Result<PathBuf, AcquisitionError> {
    if base_path.exists() {
        return Ok(base_path.to_path_buf());
    }

    // yt-dlp commonly appends the audio format extension
    let base_str = base_path.to_string_lossy();
    for ext in &["opus", "m4a", "mp3", "wav", "webm"] {
        let candidate = PathBuf::from(format!("{base_str}.{ext}"));
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(AcquisitionError::FileNotFound(base_path.to_path_buf()))
}

/// Check if the file is already Opus-encoded by probing its codec via ffprobe.
async fn is_opus(path: &Path, runner: &CommandRunner) -> bool {
    let path_str = path.to_string_lossy();
    let result = runner
        .run_checked(
            "ffprobe",
            &[
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_entries",
                "stream=codec_name",
                "-of",
                "json",
                &path_str,
            ],
        )
        .await;

    match result {
        Ok(output) => output.stdout.contains("\"opus\""),
        Err(_) => false,
    }
}

/// Probe media file for duration and size using ffprobe.
async fn probe_media(
    path: &Path,
    runner: &CommandRunner,
) -> Result<(Option<Duration>, u64), AcquisitionError> {
    let size_bytes = std::fs::metadata(path).map_err(AcquisitionError::Io)?.len();

    let path_str = path.to_string_lossy();
    let duration = match runner
        .run_checked(
            "ffprobe",
            &[
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "json",
                &path_str,
            ],
        )
        .await
    {
        Ok(output) => parse_ffprobe_duration(&output.stdout),
        Err(_) => None,
    };

    Ok((duration, size_bytes))
}

/// Parse duration from ffprobe JSON output.
///
/// Expected format: `{"format": {"duration": "123.456"}}`
fn parse_ffprobe_duration(json_str: &str) -> Option<Duration> {
    let value: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let duration_str = value.get("format")?.get("duration")?.as_str()?;
    let seconds: f64 = duration_str.parse().ok()?;
    Some(Duration::from_secs_f64(seconds))
}

/// Errors during media acquisition.
#[derive(Debug, thiserror::Error)]
pub enum AcquisitionError {
    #[error("download failed: {0}")]
    Download(String),

    #[error("transcode failed: {0}")]
    Transcode(String),

    #[error("file not found: {0}")]
    FileNotFound(PathBuf),

    #[error("unsupported source format")]
    UnsupportedFormat,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

// Allow conversion from ProcessError for ergonomic ? usage in internal helpers.
impl From<ProcessError> for AcquisitionError {
    fn from(e: ProcessError) -> Self {
        AcquisitionError::Download(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_url_accepts_http() {
        assert!(validate_url("http://example.com/video").is_ok());
    }

    #[test]
    fn test_validate_url_accepts_https() {
        assert!(validate_url("https://www.youtube.com/watch?v=abc123").is_ok());
    }

    #[test]
    fn test_validate_url_rejects_ftp() {
        assert!(validate_url("ftp://evil.com/file").is_err());
    }

    #[test]
    fn test_validate_url_rejects_command_injection() {
        assert!(validate_url("http://x.com/a; rm -rf /").is_err());
    }

    #[test]
    fn test_validate_url_rejects_empty() {
        assert!(validate_url("").is_err());
    }

    #[test]
    fn test_sanitize_filename_basic() {
        let result = sanitize_filename("https://youtube.com/watch?v=abc");
        assert!(!result.contains('/'));
        assert!(!result.contains('?'));
        assert!(!result.contains(':'));
    }

    #[test]
    fn test_sanitize_filename_prevents_path_traversal() {
        let result = sanitize_filename("../../../etc/passwd");
        assert!(!result.starts_with('.'));
        assert!(!result.contains(".."));
    }

    #[test]
    fn test_sanitize_filename_empty_fallback() {
        let result = sanitize_filename("://");
        assert_eq!(result, "download");
    }

    #[test]
    fn test_find_output_file_keeps_youtu_be_stem() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("https___youtu.be_Rqm2BJKeCgA");
        let actual = base.with_file_name("https___youtu.be_Rqm2BJKeCgA.opus");
        std::fs::write(&actual, b"").unwrap();

        let found = find_output_file(&base).unwrap();
        assert_eq!(found, actual);
    }

    #[test]
    fn test_parse_ffprobe_duration_valid() {
        let json = r#"{"format": {"duration": "123.456"}}"#;
        let duration = parse_ffprobe_duration(json).unwrap();
        assert!((duration.as_secs_f64() - 123.456).abs() < 0.001);
    }

    #[test]
    fn test_parse_ffprobe_duration_invalid_json() {
        assert!(parse_ffprobe_duration("not json").is_none());
    }

    #[test]
    fn test_parse_ffprobe_duration_missing_field() {
        let json = r#"{"format": {}}"#;
        assert!(parse_ffprobe_duration(json).is_none());
    }
}
