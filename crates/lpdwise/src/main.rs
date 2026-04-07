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
    let device = lpdwise_device::LlmfitProber
        .probe()
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
    let cut_points = lpdwise_audio::adaptive_chunk(&silences, duration_ms, size_bytes, 0);

    let chunks = if cut_points.is_empty() {
        // Single chunk — treat entire file as one chunk
        vec![lpdwise_core::AudioChunk {
            path: asset.path.clone(),
            index: 0,
            start: std::time::Duration::ZERO,
            end: asset.duration.unwrap_or(std::time::Duration::ZERO),
        }]
    } else {
        let chunk_dir = config.media_dir.join("chunks");
        std::fs::create_dir_all(&chunk_dir)?;
        lpdwise_audio::split_audio(&asset.path, &cut_points, &chunk_dir, &runner, duration_ms)
            .await
            .context("audio splitting failed")?
    };
    finish_stage(&chunk_spinner, Stage::Chunking);
    info!(chunk_count = chunks.len(), "audio chunked");

    // -- Transcribe --
    let transcribe_pb = progress::create_progress_bar(chunks.len() as u64, Stage::Transcribing);
    let mut all_segments = Vec::new();

    for chunk in &chunks {
        let segments = transcribe_chunk(engine_kind, chunk, &config)
            .await
            .with_context(|| format!("transcription failed for chunk {}", chunk.index))?;
        all_segments.extend(segments);
        transcribe_pb.inc(1);
    }
    finish_stage(&transcribe_pb, Stage::Transcribing);

    let transcript = lpdwise_core::Transcript {
        segments: all_segments,
    };

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

    eprintln!("\nPrompt 已复制到剪贴板，请粘贴到 LLM 对话窗口。");
    Ok(())
}

/// Transcribe a single audio chunk using the selected engine.
async fn transcribe_chunk(
    engine: lpdwise_core::EngineKind,
    chunk: &lpdwise_core::AudioChunk,
    config: &lpdwise_core::AppConfig,
) -> Result<Vec<lpdwise_core::TranscriptSegment>, AppError> {
    match engine {
        lpdwise_core::EngineKind::GroqWhisper => {
            let api_key = config.groq_api_key.as_deref().ok_or_else(|| {
                anyhow::anyhow!("Groq API key not configured — set GROQ_API_KEY or config file")
            })?;
            let transcript = lpdwise_asr::transcribe_chunks(std::slice::from_ref(chunk), api_key)
                .await
                .context("Groq transcription failed")?;
            Ok(transcript.segments)
        }
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
    }
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
