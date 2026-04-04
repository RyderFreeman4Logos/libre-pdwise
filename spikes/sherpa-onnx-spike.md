# Spike: sherpa-onnx Rust Integration Feasibility

**Date**: 2026-04-04
**Branch**: feat/implement-tiny
**Status**: Complete

## Summary

Evaluated three integration approaches for sherpa-onnx local ASR in libre-pdwise.
The official `sherpa-onnx` crate (published by the sherpa-onnx maintainer) is the clear winner.

## Crate Search Results

### Official Crates (k2-fsa / Fangjun Kuang)

| Crate | Version | Last Updated | Downloads | License |
|-------|---------|-------------|-----------|---------|
| `sherpa-onnx` | 1.12.34 | 2026-03-26 | 3,795 | Apache-2.0 |
| `sherpa-onnx-sys` | 1.12.34 | 2026-03-26 | 3,817 | Apache-2.0 |

- **Repository**: https://github.com/k2-fsa/sherpa-onnx (official)
- **Author**: Fangjun Kuang (@csukuangfj) — the primary maintainer of the entire sherpa-onnx project
- **Rust listed as official language**: Yes, Rust is one of 12 officially supported languages
- **Examples**: 47 Rust examples in `rust-api-examples/` covering ASR, TTS, VAD, speaker diarization, etc.

### Community Crates (all deprecated or forks)

| Crate | Version | Status | Notes |
|-------|---------|--------|-------|
| `sherpa-rs` | 0.6.8 | **DEPRECATED** | README says: "upstream now provides official Rust APIs" |
| `chobits-sherpa-rs` | 0.7.0 | Fork of sherpa-rs | Same deprecation applies |
| `lxxyx-sherpa-rs` | 0.6.8 | Fork of sherpa-rs | Same deprecation applies |
| `ly-sherpa-rs` | 0.6.10 | Fork of sherpa-rs | Same deprecation applies |
| `sherpa-transducers` | 0.5.5 | Streaming-only wrapper | Niche use case |

**Conclusion**: All community bindings point back to the official crate. No reason to use any of these.

## Official Crate Analysis

### Build Mechanism

The `sherpa-onnx-sys` crate uses a build script that:
1. Downloads precompiled sherpa-onnx libraries via `ureq` (HTTP)
2. Extracts them from `.tar.bz2` archives (via `tar` + `bzip2` crates)
3. Links statically by default (or shared via `features = ["shared"]`)

**No system dependencies required** — the build script downloads everything needed.

### Feature Flags

```toml
# Default: static linking (recommended for CLI distribution)
sherpa-onnx = { version = "1.12.34", default-features = false, features = ["static"] }

# Shared linking
sherpa-onnx = { version = "1.12.34", default-features = false, features = ["shared"] }

# With microphone support (adds cpal dependency)
sherpa-onnx = { version = "1.12.34", features = ["mic"] }
```

### API Surface (from examples)

The crate provides safe Rust wrappers for:
- `OfflineRecognizerConfig` / `OfflineRecognizer` — batch/file ASR
- `OnlineRecognizerConfig` / `OnlineRecognizer` — streaming/real-time ASR
- VAD (Voice Activity Detection) via SileroVAD
- Speaker embedding and diarization
- TTS (not needed for libre-pdwise but available)

### Supported Models

Official examples demonstrate:
- Zipformer (streaming + offline, multilingual)
- SenseVoice (multilingual)
- Whisper (offline)
- Moonshine, Nemo Parakeet, FireRedASR, Qwen3
- Paraformer

### Platform Support

- Linux x86_64 ✅
- macOS (x86_64 + aarch64) ✅
- Windows ✅
- Android arm64-v8a ✅ (key for Termux testing on Pixel 4a 5G)

### Binary Size Impact

From Android build data:
- `libonnxruntime.so`: ~15 MB
- `libsherpa-onnx-jni.so`: ~3.7 MB
- **Estimated static-linked addition to lpdwise binary**: ~15-20 MB

This is significant but acceptable for a CLI tool that bundles its own inference engine.
Model files are separate and typically 30-300 MB depending on the model.

## Compilation Test

**Environment limitation**: The current CI/development environment has a read-only cargo registry cache, preventing actual crate download and compilation. The spike project was created at `/tmp/sherpa-spike/` but could not complete `cargo check`.

**Mitigation**: The official crate has 47 working examples in the upstream repository, is at version 1.12.34 (indicating active maintenance), and the build mechanism (download precompiled libs) is well-established. Compilation risk is low.

**Recommended first action after spike approval**: Run `cargo check` in the actual project after adding the dependency, to verify compilation on the target environment.

## Option Evaluation

### Option A: Official `sherpa-onnx` crate ⭐ RECOMMENDED

| Factor | Assessment |
|--------|------------|
| Maintenance | Active — v1.12.34 updated 2026-03-26, maintained by project lead |
| API stability | High — version-locked to sherpa-onnx releases |
| Build complexity | Low — precompiled libs downloaded automatically |
| System dependencies | None — self-contained |
| Android/Termux | Supported — arm64-v8a precompiled libs available |
| Binary size | +15-20 MB (acceptable for CLI tool) |
| Performance | Native — no subprocess overhead |
| Model flexibility | All sherpa-onnx models supported |

### Option B: C API + bindgen FFI

| Factor | Assessment |
|--------|------------|
| Maintenance | High burden — must maintain bindings ourselves |
| Build complexity | High — requires CMake, C compiler toolchain |
| Justification | None — official crate already does this |

**Verdict**: Unnecessary. The official `sherpa-onnx-sys` crate already provides FFI bindings, and `sherpa-onnx` wraps them safely. Building our own would duplicate effort.

### Option C: CLI subprocess (ProcessRunner)

| Factor | Assessment |
|--------|------------|
| Maintenance | Low |
| Build complexity | None — external binary |
| System dependencies | Requires sherpa-onnx CLI binary installed |
| Binary size | Zero (external process) |
| Performance | Poor — process spawn overhead, IPC via stdout |
| Distribution | Complex — users must install sherpa-onnx separately |
| Android/Termux | Problematic — no prebuilt Linux CLI binaries in releases |

**Verdict**: Viable as fallback but significantly worse UX. Users would need to build sherpa-onnx from source or find binaries. The official Rust crate eliminates this entirely.

## Recommended Integration Architecture

```
lpdwise
├── src/
│   ├── asr/
│   │   ├── mod.rs          // AsrEngine trait
│   │   ├── groq.rs         // Groq Whisper API (cloud)
│   │   └── sherpa.rs       // sherpa-onnx (local)
│   └── ...
```

The `sherpa-onnx` crate should be an **optional feature** in `Cargo.toml`:

```toml
[features]
default = ["sherpa"]
sherpa = ["dep:sherpa-onnx"]

[dependencies]
sherpa-onnx = { version = "1.12", default-features = false, features = ["static"], optional = true }
```

This allows:
- `cargo build` — includes local ASR by default
- `cargo build --no-default-features` — cloud-only build (smaller binary)

## Risks and Mitigations

| Risk | Probability | Mitigation |
|------|-------------|------------|
| Crate download fails in restricted network | Low | Cache precompiled libs in CI, or use `shared` feature with system-installed libs |
| Android cross-compilation issues | Medium | Test early on Pixel 4a 5G; sherpa-onnx has Android-specific build scripts |
| Binary too large for Termux | Low | Use `shared` linking on Android to keep binary small |
| API breaking changes | Low | Lock to specific version; official crate follows semver |

## 采用方案

**采用方案 A**: 使用官方 `sherpa-onnx` crate (v1.12.x)。

理由：
1. 由 sherpa-onnx 项目负责人直接维护，版本与上游同步
2. 所有第三方 Rust binding 已 deprecated，统一指向官方 crate
3. 自动下载预编译库，零系统依赖
4. 支持 static/shared 链接，适配不同部署场景
5. 47 个官方 Rust 示例可作为参考
6. 支持 Android arm64，满足 Termux 测试需求

下一步行动：
1. 在 `Cargo.toml` 添加 `sherpa-onnx` 依赖（作为 optional feature）
2. 定义 `AsrEngine` trait，实现 sherpa-onnx backend
3. 在开发环境验证 `cargo check` 编译通过
4. 在 Pixel 4a 5G 上验证 `cargo build -j 1` 交叉编译
