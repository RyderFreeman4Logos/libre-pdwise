mod cli;
mod delivery;
mod doctor;
mod error;
mod input;
mod interactive;
mod progress;

use anyhow::Context;
use clap::Parser;
use tracing::{error, info};

use lpdwise_asr::AsrEngine;
use lpdwise_audio::MediaAcquirer;
use lpdwise_device::DeviceProber;
use lpdwise_process::CommandRunner;

use crate::cli::{Cli, Command, EngineArg};
use crate::error::AppError;
use crate::progress::{create_spinner, finish_stage, Stage};

#[tokio::main]
async fn main() -> Result<(), AppError> {
    init_tracing();
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Doctor) => {
            let results = doctor::run_doctor().await;
            doctor::print_doctor_results(&results);
        }
        None => {
            if let Err(e) = run_pipeline(cli).await {
                error!("{e:#}");
                eprintln!("\n错误: {e:#}");
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

/// Main extraction pipeline orchestration.
async fn run_pipeline(cli: Cli) -> Result<(), AppError> {
    // -- Load config --
    let mut config = lpdwise_core::load_config().context("failed to load config")?;

    // CLI --model-dir overrides config
    if let Some(ref model_dir) = cli.model_dir {
        config.models_dir = model_dir.clone();
    }

    // -- Resolve input --
    let source =
        input::resolve_input(cli.input.as_deref()).context("failed to resolve input source")?;
    info!(source = ?source, "input resolved");

    // -- Language selection --
    let language = if matches!(cli.language, cli::LanguageArg::Auto) && !cli.non_interactive {
        interactive::select_language().context("language selection cancelled")?
    } else {
        cli.language.to_language()
    };

    // -- Engine selection --
    let device = tokio::task::spawn_blocking(|| lpdwise_device::LlmfitProber.probe())
        .await
        .context("device detection task failed")?
        .context("device detection failed")?;

    let groq_available = config.groq_api_key.is_some();
    let recommendations = lpdwise_core::recommend_engines(language, device.ram_mb, groq_available);

    let engine_kind = match cli.engine {
        EngineArg::Auto => {
            if cli.non_interactive {
                recommendations.first().map(|r| r.engine).ok_or_else(|| {
                    anyhow::anyhow!("no ASR engine available — check `lpdwise doctor`")
                })?
            } else {
                interactive::select_engine(&recommendations)
                    .context("engine selection cancelled")?
            }
        }
        EngineArg::Groq => lpdwise_core::EngineKind::GroqWhisper,
        EngineArg::Sherpa => {
            if language == lpdwise_core::Language::Chinese {
                lpdwise_core::EngineKind::SherpaOnnxSenseVoice
            } else {
                lpdwise_core::EngineKind::SherpaOnnxWhisper
            }
        }
    };
    info!(engine = ?engine_kind, "engine selected");

    // -- Template selection --
    let template = if cli.non_interactive {
        cli.template.to_template()
    } else {
        interactive::select_template().context("template selection cancelled")?
    };

    // -- Acquire media --
    let acquire_spinner = create_spinner(Stage::Acquiring);
    let asset = match &source {
        lpdwise_core::InputSource::Url(_) => {
            let acquirer = lpdwise_audio::YtDlpAcquirer::new(config.media_dir.clone());
            acquirer
                .acquire(source.clone())
                .await
                .context("media acquisition failed")?
        }
        lpdwise_core::InputSource::File(path) => {
            let opus_path = config
                .media_dir
                .join(
                    path.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .as_ref(),
                )
                .with_extension("opus");
            let runner = CommandRunner::with_default_timeout();
            let transcoded = lpdwise_audio::transcode_to_opus(path, &opus_path, &runner)
                .await
                .context("audio transcoding failed")?;
            transcoded
        }
    };
    finish_stage(&acquire_spinner, Stage::Acquiring);

    // -- Detect silences and chunk --
    let chunk_spinner = create_spinner(Stage::Chunking);
    let runner = CommandRunner::with_default_timeout();
    let silences = lpdwise_audio::detect_silences(&asset.path, &runner)
        .await
        .context("silence detection failed")?;

    let duration_ms = asset.duration.map(|d| d.as_millis() as u64).unwrap_or(0);
    let size_bytes = asset.size_bytes.unwrap_or(0);
    let groq_chunk_policy = if matches!(engine_kind, lpdwise_core::EngineKind::GroqWhisper) {
        build_groq_chunk_policy(&cli)?
    } else {
        lpdwise_audio::ChunkingPolicy::groq_whisper()
    };
    let cut_points = if matches!(engine_kind, lpdwise_core::EngineKind::GroqWhisper) {
        lpdwise_audio::adaptive_chunk_with_policy(
            &silences,
            duration_ms,
            size_bytes,
            groq_chunk_policy,
        )
    } else {
        lpdwise_audio::adaptive_chunk(&silences, duration_ms, size_bytes, 0)
    };

    let chunks = if cut_points.is_empty() {
        // Single chunk — treat entire file as one chunk
        vec![lpdwise_core::AudioChunk {
            path: asset.path.clone(),
            index: 0,
            audio_start: std::time::Duration::ZERO,
            start: std::time::Duration::ZERO,
            end: asset.duration.unwrap_or(std::time::Duration::ZERO),
        }]
    } else {
        let chunk_dir = config.media_dir.join("chunks");
        std::fs::create_dir_all(&chunk_dir)?;
        if matches!(engine_kind, lpdwise_core::EngineKind::GroqWhisper) {
            lpdwise_audio::split_audio_with_overlap(
                &asset.path,
                &cut_points,
                &chunk_dir,
                &runner,
                duration_ms,
                groq_chunk_policy.overlap_ms,
            )
            .await
            .context("audio splitting failed")?
        } else {
            lpdwise_audio::split_audio(&asset.path, &cut_points, &chunk_dir, &runner, duration_ms)
                .await
                .context("audio splitting failed")?
        }
    };
    finish_stage(&chunk_spinner, Stage::Chunking);
    info!(chunk_count = chunks.len(), "audio chunked");

    // -- Transcribe --
    let transcribe_pb = progress::create_progress_bar(chunks.len() as u64, Stage::Transcribing);
    let transcript = match engine_kind {
        lpdwise_core::EngineKind::GroqWhisper => {
            let transcript = transcribe_groq_chunks(&chunks, &config, language, &cli)
                .await
                .context("Groq transcription failed")?;
            transcribe_pb.inc(chunks.len() as u64);
            transcript
        }
        lpdwise_core::EngineKind::SherpaOnnxSenseVoice
        | lpdwise_core::EngineKind::SherpaOnnxWhisper => {
            let mut all_segments = Vec::new();
            for chunk in &chunks {
                let segments = transcribe_chunk(engine_kind, chunk, &config)
                    .await
                    .with_context(|| format!("transcription failed for chunk {}", chunk.index))?;
                lpdwise_asr::merge_chunk_segments(&mut all_segments, chunk, segments);
                transcribe_pb.inc(1);
            }

            lpdwise_core::Transcript {
                segments: all_segments,
            }
        }
    };
    finish_stage(&transcribe_pb, Stage::Transcribing);

    // -- Assemble prompt --
    let assemble_spinner = create_spinner(Stage::Assembling);
    let payload = template.render(&transcript, language);
    finish_stage(&assemble_spinner, Stage::Assembling);

    // -- Deliver --
    let deliver_spinner = create_spinner(Stage::Delivering);
    let clipboard = lpdwise_clipboard::auto_detect();
    let archive_dir = if cli.no_archive {
        None
    } else {
        Some(config.archive_dir.as_path())
    };
    delivery::deliver(
        &payload,
        clipboard.as_ref(),
        &source,
        &transcript,
        archive_dir,
    )
    .context("delivery failed")?;
    finish_stage(&deliver_spinner, Stage::Delivering);

    if !cli.no_archive {
        eprintln!("归档完成: {}", config.archive_dir.display());
    }

    eprintln!("\nPrompt 已发送到可用的剪贴板后端；若无系统剪贴板，则已输出到标准输出。");
    Ok(())
}

/// Transcribe a single audio chunk using the selected engine.
async fn transcribe_chunk(
    engine: lpdwise_core::EngineKind,
    chunk: &lpdwise_core::AudioChunk,
    config: &lpdwise_core::AppConfig,
) -> Result<Vec<lpdwise_core::TranscriptSegment>, AppError> {
    match engine {
        lpdwise_core::EngineKind::SherpaOnnxSenseVoice
        | lpdwise_core::EngineKind::SherpaOnnxWhisper => {
            let model_dir = config.models_dir.clone();
            let sherpa = lpdwise_asr::SherpaOnnxEngine::new(model_dir);
            let segments = sherpa
                .transcribe(chunk)
                .await
                .context("sherpa-onnx transcription failed")?;
            Ok(segments)
        }
        lpdwise_core::EngineKind::GroqWhisper => Err(anyhow::anyhow!(
            "Groq transcription should be handled through sequential chunk processing"
        )),
    }
}

async fn transcribe_groq_chunks(
    chunks: &[lpdwise_core::AudioChunk],
    config: &lpdwise_core::AppConfig,
    language: lpdwise_core::Language,
    cli: &Cli,
) -> Result<lpdwise_core::Transcript, AppError> {
    let api_key = config.groq_api_key.as_deref().ok_or_else(|| {
        anyhow::anyhow!("Groq API key not configured — set GROQ_API_KEY or config file")
    })?;
    let options = lpdwise_asr::GroqTranscriptionOptions::new(Some(language.bcp47_tag()))
        .with_prompt_max_chars(cli.groq.prompt_chars);

    lpdwise_asr::transcribe_chunks_with_options(chunks, api_key, options)
        .await
        .context("Groq sequential transcription failed")
}

fn build_groq_chunk_policy(cli: &Cli) -> Result<lpdwise_audio::ChunkingPolicy, AppError> {
    if cli.groq.min_chunk_seconds == 0 {
        return Err(anyhow::anyhow!(
            "--min-chunk-seconds must be greater than 0"
        ));
    }
    if cli.groq.min_chunk_seconds > cli.groq.target_chunk_seconds {
        return Err(anyhow::anyhow!(
            "--min-chunk-seconds must be <= --target-chunk-seconds"
        ));
    }
    if cli.groq.target_chunk_seconds > cli.groq.max_chunk_seconds {
        return Err(anyhow::anyhow!(
            "--target-chunk-seconds must be <= --max-chunk-seconds"
        ));
    }
    if cli.groq.overlap_seconds >= cli.groq.max_chunk_seconds {
        return Err(anyhow::anyhow!(
            "--overlap-seconds must be smaller than --max-chunk-seconds"
        ));
    }

    let mut policy = lpdwise_audio::ChunkingPolicy::groq_whisper();
    policy.max_chunk_duration_ms = cli.groq.max_chunk_seconds.saturating_mul(1000);
    policy.target_chunk_duration_ms = cli.groq.target_chunk_seconds.saturating_mul(1000);
    policy.min_chunk_duration_ms = cli.groq.min_chunk_seconds.saturating_mul(1000);
    policy.overlap_ms = cli.groq.overlap_seconds.saturating_mul(1000);
    Ok(policy)
}

/// Initialize tracing with env filter support.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_cli(args: &[&str]) -> Cli {
        Cli::parse_from(args.iter().copied())
    }

    #[test]
    fn test_build_groq_chunk_policy_maps_cli_values() {
        let cli = parse_cli(&[
            "lpdwise",
            "--target-chunk-seconds",
            "240",
            "--max-chunk-seconds",
            "480",
            "--min-chunk-seconds",
            "75",
            "--overlap-seconds",
            "12",
            "https://example.com",
        ]);

        let policy = build_groq_chunk_policy(&cli).expect("valid Groq chunk policy");

        assert_eq!(policy.target_chunk_duration_ms, 240_000);
        assert_eq!(policy.max_chunk_duration_ms, 480_000);
        assert_eq!(policy.min_chunk_duration_ms, 75_000);
        assert_eq!(policy.overlap_ms, 12_000);
    }

    #[test]
    fn test_build_groq_chunk_policy_rejects_zero_minimum() {
        let cli = parse_cli(&["lpdwise", "--min-chunk-seconds", "0", "https://example.com"]);

        let error = build_groq_chunk_policy(&cli).expect_err("zero minimum must fail");

        assert!(error
            .to_string()
            .contains("--min-chunk-seconds must be greater than 0"));
    }

    #[test]
    fn test_build_groq_chunk_policy_rejects_minimum_above_target() {
        let cli = parse_cli(&[
            "lpdwise",
            "--target-chunk-seconds",
            "120",
            "--min-chunk-seconds",
            "121",
            "https://example.com",
        ]);

        let error = build_groq_chunk_policy(&cli).expect_err("minimum above target must fail");

        assert!(error
            .to_string()
            .contains("--min-chunk-seconds must be <= --target-chunk-seconds"));
    }

    #[test]
    fn test_build_groq_chunk_policy_rejects_target_above_maximum() {
        let cli = parse_cli(&[
            "lpdwise",
            "--target-chunk-seconds",
            "601",
            "--max-chunk-seconds",
            "600",
            "https://example.com",
        ]);

        let error = build_groq_chunk_policy(&cli).expect_err("target above maximum must fail");

        assert!(error
            .to_string()
            .contains("--target-chunk-seconds must be <= --max-chunk-seconds"));
    }

    #[test]
    fn test_build_groq_chunk_policy_rejects_overlap_at_or_above_maximum() {
        let cli = parse_cli(&[
            "lpdwise",
            "--target-chunk-seconds",
            "120",
            "--max-chunk-seconds",
            "120",
            "--overlap-seconds",
            "120",
            "https://example.com",
        ]);

        let error = build_groq_chunk_policy(&cli).expect_err("overlap at max must fail");

        assert!(error
            .to_string()
            .contains("--overlap-seconds must be smaller than --max-chunk-seconds"));
    }
}
