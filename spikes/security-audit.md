# Security Audit Checklist

**Date**: 2026-04-04
**Branch**: feat/implement-tiny
**Auditor**: automated + manual review

## 1. URL Input Whitelist

**Status**: PASS

`crates/lpdwise-audio/src/acquisition.rs:148-157` — `validate_url()` uses regex `^https?://[^\s]+$` to whitelist only HTTP(S) URLs. Rejects `ftp://`, `javascript:`, `file://`, `data:`, empty strings, and URLs with embedded whitespace.

**Tests**: 5 unit tests in `acquisition.rs` + 6 integration tests in `tests/url_security.rs`.

## 2. API Key Not in Tracing Output

**Status**: PASS

- `crates/lpdwise-asr/src/groq.rs:43` — `#[instrument(skip(self, audio_path))]` on `call_api()`: `self` (which contains `api_key`) is skipped.
- `crates/lpdwise-asr/src/groq.rs:125` — `#[instrument(skip(self))]` on `transcribe()`: `self` skipped.
- `crates/lpdwise-asr/src/groq.rs:162` — `#[instrument(skip_all)]` on `transcribe_chunks()`: all params including `api_key` skipped.
- `crates/lpdwise-asr/src/model.rs:31` — `#[instrument(skip_all)]` on `download_model()`: no secrets in scope.

No `api_key` field appears in any `tracing::info!`, `debug!`, or `warn!` format string.

## 3. Command::arg() — No Shell Splicing

**Status**: PASS

All subprocess invocations use `Command::new(program).args(&[...])` with individual arguments. No `sh -c` with string interpolation from user input.

Locations:
- `crates/lpdwise-process/src/runner.rs:64` — `Command::new(program).args(args)` (tokio)
- `crates/lpdwise-device/src/llmfit.rs:32` — `Command::new("llmfit").args(...)` (std)
- `crates/lpdwise-clipboard/src/termux.rs:18,36` — `Command::new("termux-clipboard-{get,set}")` (std)

**Note**: `runner.rs:377` test uses `sh -c "echo oops >&2; exit 1"` but this is in `#[cfg(test)]` only and not user-facing.

## 4. Termux Clipboard via stdin

**Status**: PASS

`crates/lpdwise-clipboard/src/termux.rs:36-46` — `termux-clipboard-set` receives content via `stdin.write_all()`, not via command-line arguments. This prevents sensitive content from appearing in `/proc/*/cmdline` or `ps` output.

Code comment at line 34-35 explicitly documents this design choice:
> "Write content via stdin pipe instead of command-line argument to prevent sensitive data from appearing in process listings."

## 5. Temporary File RAII Cleanup

**Status**: PASS

All `tempfile` usage is in test code only (dev-dependencies). Production code does not create temporary files directly — yt-dlp output goes to the configured `media_dir` and is managed by the application lifecycle.

Test locations use `tempfile::TempDir` and `NamedTempFile` which auto-delete on Drop:
- `crates/lpdwise-core/src/config.rs` (3 tests)
- `crates/lpdwise-process/src/runner.rs` (1 test)
- `crates/lpdwise-asr/src/model.rs` (2 tests)
- `crates/lpdwise-archive/src/git.rs` (3 tests)
- `crates/lpdwise-audio/src/chunker.rs` (2 tests)
- `crates/lpdwise/src/delivery.rs` (1 test)

## 6. yt-dlp Filename Sanitize

**Status**: PASS

`crates/lpdwise-audio/src/acquisition.rs:160-179` — `sanitize_filename()` replaces all characters except `[a-zA-Z0-9._-]` with `_`, then strips leading dots and underscores to prevent path traversal. Falls back to `"download"` if the result is empty.

**Tests**: 3 unit tests covering basic sanitization, path traversal prevention (`../../../etc/passwd`), and empty-input fallback.

## 7. Model File SHA256 Verification

**Status**: PASS

`crates/lpdwise-asr/src/model.rs:80-92` — `download_model()` verifies downloaded bytes against `expected_sha256` using `sha2::Sha256` before writing to disk. On mismatch, returns `ModelError::ChecksumMismatch` with both expected and actual hashes.

A `.sha256_verified` marker file is written only after successful verification (line 102), preventing re-download of verified models.

**Tests**: `test_sha256_file` verifies hash computation, `test_download_model_skip_if_verified` verifies the cache-skip path.

## Summary

| # | Check | Status | Location |
|---|-------|--------|----------|
| 1 | URL whitelist | PASS | `acquisition.rs:148` |
| 2 | API key not traced | PASS | `groq.rs:43,125,162` |
| 3 | No shell splicing | PASS | `runner.rs:64`, `termux.rs:18,36` |
| 4 | Termux stdin pipe | PASS | `termux.rs:36-46` |
| 5 | Temp file RAII | PASS | test-only usage |
| 6 | Filename sanitize | PASS | `acquisition.rs:160` |
| 7 | SHA256 verification | PASS | `model.rs:80-92` |
