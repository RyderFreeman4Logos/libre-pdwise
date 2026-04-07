use std::path::PathBuf;
use std::time::Duration;

use lpdwise_core::types::{AudioChunk, TranscriptSegment};
use lpdwise_process::{CommandRunner, ProcessError, ProcessRunner};
use tracing::{debug, instrument};

use crate::engine::{AsrEngine, AsrError};

const SHERPA_CLI: &str = "sherpa-onnx-offline";
const SHERPA_MISE_SPEC: &str = "github:k2-fsa/sherpa-onnx";
const SHERPA_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Default HuggingFace model identifier for sherpa-onnx transducer.
const DEFAULT_MODEL_NAME: &str =
    "csukuangfj/sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20";

/// ASR engine backed by sherpa-onnx for local on-device inference.
///
/// Uses the sherpa-onnx CLI as a subprocess. If the CLI binary is not
/// installed, returns `AsrError::NotAvailable` with installation instructions.
pub struct SherpaOnnxEngine {
    model_dir: PathBuf,
    runner: CommandRunner,
}

impl SherpaOnnxEngine {
    pub fn new(model_dir: PathBuf) -> Self {
        Self {
            model_dir,
            runner: CommandRunner::new(Duration::from_secs(10 * 60)),
        }
    }

    /// Ensure model files exist in `model_dir`, downloading if necessary.
    async fn ensure_model(&self) -> Result<(), AsrError> {
        if self.find_model_files().is_ok() {
            return Ok(());
        }

        debug!(model_dir = %self.model_dir.display(), "model files not found, downloading");

        crate::model::download_model(DEFAULT_MODEL_NAME, &self.model_dir, None)
            .await
            .map_err(|e| {
                AsrError::ModelLoad(format!(
                    "failed to download model to {}: {e}",
                    self.model_dir.display()
                ))
            })?;

        Ok(())
    }

    /// Resolve the sherpa-onnx executable, preferring the mise-managed binary
    /// when available so PATH/shim mismatches do not break transcription.
    async fn resolve_cli(&self) -> Result<String, AsrError> {
        let mut failures = Vec::new();

        if let Some(path) = self.resolve_mise_binary().await {
            match self.probe_cli_candidate(path.as_str()).await {
                Ok(()) => return Ok(path),
                Err(reason) => failures.push(format!("mise-managed {SHERPA_CLI}: {reason}")),
            }
        }
        failures.push(
            "`mise which sherpa-onnx-offline` did not resolve a healthy active binary".to_string(),
        );

        match self.probe_cli_candidate(SHERPA_CLI).await {
            Ok(()) => return Ok(SHERPA_CLI.to_string()),
            Err(reason) => failures.push(format!("PATH {SHERPA_CLI}: {reason}")),
        }

        debug!("sherpa-onnx unavailable, attempting to install/activate via mise use -g...");
        match self
            .runner
            .run_streaming_visible("mise", &["use", "-g", SHERPA_MISE_SPEC], None)
            .await
        {
            Ok(_) => {
                if let Some(path) = self.resolve_mise_binary().await {
                    match self.probe_cli_candidate(path.as_str()).await {
                        Ok(()) => return Ok(path),
                        Err(reason) => failures
                            .push(format!("mise-managed {SHERPA_CLI} after install: {reason}")),
                    }
                }
                failures.push(format!(
                    "`mise use -g {SHERPA_MISE_SPEC}` succeeded but no healthy active {SHERPA_CLI} binary was resolved"
                ));
            }
            Err(ProcessError::NonZeroExit { status, .. }) => failures.push(format!(
                "`mise use -g {SHERPA_MISE_SPEC}` exited with {status}"
            )),
            Err(e) => failures.push(format!(
                "failed to run `mise use -g {SHERPA_MISE_SPEC}`: {e}"
            )),
        }

        Err(AsrError::NotAvailable(format!(
            "{SHERPA_CLI} unavailable after PATH and mise resolution attempts: {}. \
             Install sherpa-onnx with `mise use -g {SHERPA_MISE_SPEC}` or follow \
             https://k2-fsa.github.io/sherpa/onnx/install.html",
            failures.join(" | ")
        )))
    }

    async fn resolve_mise_binary(&self) -> Option<String> {
        let output = self
            .runner
            .run_checked("mise", &["which", SHERPA_CLI])
            .await
            .ok()?;
        let path = output.stdout.trim();
        if path.is_empty() {
            None
        } else {
            Some(path.to_string())
        }
    }

    async fn probe_cli_candidate(&self, program: &str) -> Result<(), String> {
        let probe_runner = CommandRunner::new(SHERPA_PROBE_TIMEOUT);
        match probe_runner.run_checked(program, &["--help"]).await {
            Ok(_) | Err(ProcessError::NonZeroExit { .. }) => Ok(()),
            Err(ProcessError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                Err("not found".to_string())
            }
            Err(ProcessError::Timeout(timeout)) => {
                Err(format!("timed out during --help probe after {timeout:?}"))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Find the encoder, decoder, joiner, and tokens files in model_dir.
    fn find_model_files(&self) -> Result<SherpaModelFiles, AsrError> {
        let dir = &self.model_dir;

        let find_file = |patterns: &[&str]| -> Result<PathBuf, AsrError> {
            for pattern in patterns {
                let path = dir.join(pattern);
                if path.exists() {
                    return Ok(path);
                }
            }
            Err(AsrError::ModelLoad(format!(
                "model file matching {:?} not found in {}",
                patterns,
                dir.display()
            )))
        };

        // sherpa-onnx models vary in naming; try common patterns
        let encoder = find_file(&[
            "encoder-epoch-99-avg-1.onnx",
            "encoder.onnx",
            "encoder-epoch-99-avg-1.int8.onnx",
            "encoder.int8.onnx",
        ])?;

        let decoder = find_file(&[
            "decoder-epoch-99-avg-1.onnx",
            "decoder.onnx",
            "decoder-epoch-99-avg-1.int8.onnx",
            "decoder.int8.onnx",
        ])?;

        let joiner = find_file(&[
            "joiner-epoch-99-avg-1.onnx",
            "joiner.onnx",
            "joiner-epoch-99-avg-1.int8.onnx",
            "joiner.int8.onnx",
        ])?;

        let tokens = find_file(&["tokens.txt"])?;

        Ok(SherpaModelFiles {
            encoder,
            decoder,
            joiner,
            tokens,
        })
    }
}

struct SherpaModelFiles {
    encoder: PathBuf,
    decoder: PathBuf,
    joiner: PathBuf,
    tokens: PathBuf,
}

impl AsrEngine for SherpaOnnxEngine {
    #[instrument(skip(self), fields(chunk_index = chunk.index, model_dir = %self.model_dir.display()))]
    async fn transcribe(&self, chunk: &AudioChunk) -> Result<Vec<TranscriptSegment>, AsrError> {
        let cmd = self.resolve_cli().await?;
        self.ensure_model().await?;

        let model_files = self.find_model_files()?;

        let args = vec![
            "--encoder".to_string(),
            model_files.encoder.to_string_lossy().into_owned(),
            "--decoder".to_string(),
            model_files.decoder.to_string_lossy().into_owned(),
            "--joiner".to_string(),
            model_files.joiner.to_string_lossy().into_owned(),
            "--tokens".to_string(),
            model_files.tokens.to_string_lossy().into_owned(),
            chunk.path.to_string_lossy().into_owned(),
        ];

        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let output = self
            .runner
            .run_checked(&cmd, &arg_refs)
            .await
            .map_err(|e| match e {
                ProcessError::Io(io) => AsrError::Io(io),
                other => AsrError::ApiRequest(format!("sherpa-onnx CLI failed: {other}")),
            })?;

        // sherpa-onnx outputs the transcription text to stdout.
        // Parse it as a single segment spanning the chunk duration.
        let text = output.stdout.trim().to_string();

        if text.is_empty() {
            debug!("sherpa-onnx returned empty transcription");
            return Ok(Vec::new());
        }

        debug!(text_len = text.len(), "sherpa-onnx transcribed chunk");

        Ok(vec![TranscriptSegment {
            text,
            start: chunk.start,
            end: chunk.end,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sherpa_engine_creation() {
        let engine = SherpaOnnxEngine::new(PathBuf::from("/models/whisper"));
        assert_eq!(engine.model_dir, PathBuf::from("/models/whisper"));
    }

    #[test]
    fn test_model_files_missing() {
        let engine = SherpaOnnxEngine::new(PathBuf::from("/nonexistent"));
        let result = engine.find_model_files();
        assert!(result.is_err());
    }

    #[test]
    fn test_model_files_found() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        // Create dummy model files
        std::fs::write(dir.join("encoder.onnx"), b"").unwrap();
        std::fs::write(dir.join("decoder.onnx"), b"").unwrap();
        std::fs::write(dir.join("joiner.onnx"), b"").unwrap();
        std::fs::write(dir.join("tokens.txt"), b"").unwrap();

        let engine = SherpaOnnxEngine::new(dir.to_path_buf());
        let files = engine.find_model_files().unwrap();

        assert!(files.encoder.ends_with("encoder.onnx"));
        assert!(files.decoder.ends_with("decoder.onnx"));
        assert!(files.joiner.ends_with("joiner.onnx"));
        assert!(files.tokens.ends_with("tokens.txt"));
    }
}
