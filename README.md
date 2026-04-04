# libre-pdwise

BYOB (Bring Your Own Brain) CLI tool for audio/video knowledge extraction. Extracts speech from media, assembles expert prompts, and delivers them to your clipboard — ready for any LLM.

## How It Works

```
YouTube URL / local file
        │
        ▼
   yt-dlp + ffmpeg (audio extraction)
        │
        ▼
   ASR engine (Groq Whisper API or sherpa-onnx local)
        │
        ▼
   Prompt assembly (language-aware, tail-placement)
        │
        ▼
   Clipboard / git archive
```

## Installation

```bash
# via cargo-binstall (prebuilt binary)
cargo binstall lpdwise

# or build from source
cargo install --path crates/lpdwise
```

## Quick Start

```bash
# Check that dependencies are available
lpdwise doctor

# Extract knowledge from a YouTube video
lpdwise https://www.youtube.com/watch?v=...

# Extract from a local audio file
lpdwise /path/to/recording.mp3
```

## Configuration

Configuration file: `~/.config/libre-pdwise/config.toml`

```toml
groq_api_key = "gsk_..."
# data_dir = "/custom/path"   # optional override
```

### Environment Variables

| Variable | Description | Priority |
|----------|-------------|----------|
| `GROQ_API_KEY` | Groq Whisper API key | Highest (overrides config file) |
| `LPDWISE_DATA_DIR` | Data directory override | Highest |
| `HF_MIRROR` | HuggingFace mirror URL for model downloads | - |

### Data Directory Layout

```
~/.local/share/libre-pdwise/
├── media/       # downloaded audio
├── archive/     # git-based transcript archive
├── models/      # sherpa-onnx model files
└── logs/        # structured logs
```

## ASR Engines

| Engine | Type | Best For | Requires |
|--------|------|----------|----------|
| Groq Whisper | Cloud API | English, fast turnaround | `GROQ_API_KEY` |
| sherpa-onnx SenseVoice | Local | Chinese speech | 2GB+ RAM |
| sherpa-onnx Whisper | Local | General fallback | 2GB+ RAM |

Engine selection is automatic based on detected language and available resources.

## Prompt Templates

| Template | Purpose |
|----------|---------|
| Standard | Structured summary with outline, key points, and source quotes |
| Contrarian | Extract counterintuitive claims with evidence assessment |
| Political | Political-economic logic decomposition: actors, interests, game theory |
| Translation | Full faithful translation into Chinese with terminology annotations |

## Privacy

- **Cloud ASR** (Groq): audio is sent to Groq's API for transcription. See [Groq's privacy policy](https://groq.com/privacy-policy/).
- **Local ASR** (sherpa-onnx): all processing happens on-device. No data leaves your machine.
- **Archive**: transcripts are saved locally via git. Opt out with `--no-archive`.
- **No telemetry**: libre-pdwise does not collect or transmit usage data.

## License

MIT
