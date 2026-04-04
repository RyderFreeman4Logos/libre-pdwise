# Spike: cargo-zigbuild Cross-Platform Compatibility

**Date**: 2026-04-04  
**Branch**: feat/implement-tiny  
**Scope**: Documentation research only. No local or CI build was run for this spike.

## Summary

`cargo-zigbuild` is a viable path for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, and `aarch64-apple-darwin`, but the risk profile is not the same across targets:

- `x86_64-unknown-linux-gnu`: strongest path; expected to work well with vendored `git2` and the prebuilt `sherpa-onnx` archives.
- `aarch64-unknown-linux-gnu`: also strong; `sherpa-onnx-sys` ships dedicated Linux aarch64 archives, and `git2` can avoid host libgit2/OpenSSL friction if features are chosen carefully.
- `aarch64-apple-darwin`: feasible, but this is the most conditional target. `cargo-zigbuild` can target macOS, yet Zig still needs a macOS SDK for Darwin frameworks. Both `git2` and `arboard` link Apple frameworks, so CI must provide `SDKROOT` or use the official cargo-zigbuild container image that already includes a macOS SDK.

## Target-by-Target Assessment

| Target | cargo-zigbuild outlook | `git2` outlook | `arboard` outlook | `sherpa-onnx` outlook | Expected compatibility |
|---|---|---|---|---|---|
| `x86_64-unknown-linux-gnu` | Mature, officially supported by cargo-zigbuild | Good with vendored features | Good for compile; runtime depends on X11/Wayland environment | Good; upstream crate ships prebuilt Linux x64 archives | **High** |
| `aarch64-unknown-linux-gnu` | Mature, officially supported by cargo-zigbuild | Good with vendored features | Good for compile; runtime depends on X11/Wayland environment | Good; upstream crate ships prebuilt Linux aarch64 archives | **High** |
| `aarch64-apple-darwin` | Supported, but requires macOS SDK for framework linking | Medium; vendored libgit2 still links Apple frameworks | Medium; depends on AppKit/CoreFoundation/CoreGraphics and therefore on SDK availability | Good; upstream crate ships prebuilt macOS arm64 archives | **Medium** |

## cargo-zigbuild Findings

### What cargo-zigbuild clearly supports

The upstream README says:

- only Linux and macOS targets are currently supported;
- Zig is used only when an explicit `--target` is passed;
- the project provides a Docker image with a macOS SDK already installed;
- Apple cross-compilation still needs `SDKROOT` because Zig does not automatically solve Darwin framework linking on its own.

For Linux GNU targets, the README also recommends pinning the minimum glibc version by suffixing the target triple, for example:

```bash
cargo zigbuild --target x86_64-unknown-linux-gnu.2.17
cargo zigbuild --target aarch64-unknown-linux-gnu.2.17
```

That is useful for release binaries because Zig otherwise defaults to a glibc baseline chosen by the bundled Zig version.

### Important limitations

- Do not rely on `cargo zigbuild` without `--target`; that falls back to normal `cargo build`.
- Do not plan around `-C target-feature=+crt-static` on `*-gnu`; the upstream README says static glibc linking is not supported.
- If a crate needs headers or system libraries outside Zig's sysroot, `CFLAGS` and `RUSTFLAGS` may need explicit search paths.

## `git2` Compatibility Findings

### Does `vendored-libgit2` solve cross-compilation?

Partially, not completely.

`git2` enables `https` and `ssh` by default. The docs.rs feature page shows:

- `default` enables `https` and `ssh`;
- `vendored-libgit2` maps to `libgit2-sys/vendored`;
- `vendored-openssl` maps to both `libgit2-sys/vendored-openssl` and `openssl-sys/vendored`.

This means `vendored-libgit2` solves the "find a compatible system `libgit2`" part, but it does **not** by itself remove the OpenSSL/libssh2 surface added by default networking features.

### Recommended feature strategy

If libre-pdwise only needs local repository access:

```toml
git2 = { version = "0.20", default-features = false, features = ["vendored-libgit2"] }
```

That is the lowest-risk cross-compilation setup.

If HTTPS is required:

```toml
git2 = { version = "0.20", default-features = false, features = ["https", "vendored-libgit2", "vendored-openssl"] }
```

If SSH is also required:

```toml
git2 = { version = "0.20", default-features = false, features = ["https", "ssh", "vendored-libgit2", "vendored-openssl"] }
```

### Platform notes

- Linux: `libgit2-sys` uses OpenSSL for HTTPS on non-Apple Unix targets, so `vendored-openssl` is the safest default when network features are enabled.
- macOS: `libgit2-sys` uses Secure Transport and CommonCrypto on Apple targets, not OpenSSL, but its build still links `Security` and `CoreFoundation`. That means Apple cross-compilation still depends on a valid macOS SDK.

**Inference**: for this project, `default-features = false` is more important than `vendored-libgit2` alone. Without that change, cross-compilation risk stays higher than necessary.

## `arboard` Cross-Compilation Findings

### Linux backends

`arboard`'s README says Linux defaults to the X11/XWayland backend. Wayland support exists, but users must enable the `wayland-data-control` feature explicitly.

The crate's manifest shows:

- Linux uses `x11rb` by default;
- Wayland support is optional through `wl-clipboard-rs`;
- macOS uses Objective-C/AppKit/CoreFoundation/CoreGraphics bindings.

### What this means for cross-compilation

Compile-time risk is moderate, but runtime risk is higher than compile-time risk:

- On Linux, `arboard`'s backend dependencies are Rust crates, so it is less dependent on system `-dev` packages than older clipboard crates.
- At runtime, clipboard access still depends on the actual desktop environment:
  - X11 or XWayland for the default backend;
  - `wayland-data-control` support in the compositor for native Wayland mode.

The README explicitly warns that:

- pure Wayland environments may fail if the compositor does not expose the required data-control protocols;
- XWayland may still be necessary as a fallback;
- sandboxed environments need the X11 socket exposed in addition to the Wayland interface.

### CLI-specific caution

The README also documents clipboard ownership semantics on Linux: the source process may need to stay alive long enough for the data to be requested, or call `wait()` when setting clipboard contents. For a short-lived CLI, this matters as much as cross-compilation itself.

### Recommended feature strategy

- If Linux clipboard support matters on modern desktops, enable:

```toml
arboard = { version = "3.6", features = ["wayland-data-control"] }
```

- If the product can tolerate "clipboard unsupported in some pure Wayland sessions", the default feature set is simpler.
- For `aarch64-apple-darwin`, expect the crate to compile only when the macOS SDK is available because AppKit/CoreFoundation/CoreGraphics are Apple framework dependencies.

## `sherpa-onnx` and ONNX Runtime Findings

### What the Rust crate supports directly

The current `sherpa-onnx-sys` build script selects prebuilt archives for:

- `linux` + `x86_64`
- `linux` + `aarch64`
- `macos` + `x86_64`
- `macos` + `aarch64`

for both static and shared link modes.

That is strong evidence that the Rust crate itself is intended to support all three targets needed by libre-pdwise.

Recommended dependency shape:

```toml
sherpa-onnx = { version = "1.12", default-features = false, features = ["static"] }
```

Why `static` first:

- it avoids shipping a separate `onnxruntime` dynamic library in the simplest release path;
- the build script still knows how to link the required platform runtimes.

### Remaining platform-specific details

Even in static mode, the build script still links platform runtime components:

- Linux: `stdc++`, `m`, `pthread`, `dl`
- macOS: `c++` and `Foundation`

So this is not "zero platform dependency", but it is much better than building ONNX Runtime from source in CI.

### ONNX Runtime upstream status

The official ONNX Runtime compatibility page still describes a conservative platform matrix and is not the clearest source for Apple Silicon packaging status. However, the ONNX Runtime v1.23.0 release notes say the **next** release will stop providing `x86_64` binaries for macOS and iOS. That implies macOS support is continuing in the arm64 direction rather than disappearing.

**Inference**: for Apple Silicon, sherpa-onnx's prebuilt `osx-arm64` archives are a stronger operational signal than the higher-level ONNX Runtime compatibility page.

### Expected compatibility for libre-pdwise

- `x86_64-unknown-linux-gnu`: good
- `aarch64-unknown-linux-gnu`: good
- `aarch64-apple-darwin`: good if the macOS SDK is present in CI

The main risk is not ONNX Runtime ABI support; it is CI environment setup.

## Recommended CI Workflow

## 1. Build on `ubuntu-latest` with an explicit target matrix

Suggested matrix:

- `x86_64-unknown-linux-gnu.2.17`
- `aarch64-unknown-linux-gnu.2.17`
- `aarch64-apple-darwin`

Using `.2.17` for Linux increases compatibility with older glibc systems. If newer distro support is acceptable, `.2.28` matches current Zig defaults more closely.

## 2. Use cargo-zigbuild's container image for Darwin jobs

For Apple targets from Linux CI, prefer the official image because the upstream README says it already contains a macOS SDK. That removes the hardest part of Darwin cross-linking.

If the pipeline does not use the container image, it must set `SDKROOT` explicitly.

## 3. Keep features explicit in CI

Do not let default features hide portability problems. Use explicit feature sets in at least one CI job, for example:

```bash
cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.17 --no-default-features --features "git2-vendored,sherpa-static"
```

The exact feature names should match the project's future Cargo feature design, but the principle is important: test the minimal portable build, not only the most convenient local build.

## 4. Cache Rust artifacts, but treat downloaded native archives as disposable

Good CI hygiene:

- cache Cargo registry, git index, and `target/`;
- allow `sherpa-onnx-sys` to re-download its native bundle when needed;
- avoid over-engineering a custom native archive cache until build times justify it.

## 5. Separate "compile" from "package"

Recommended stages:

1. compile matrix with `cargo zigbuild`
2. smoke-test Linux binaries on Linux runners
3. package artifacts
4. optionally notarization/signing later for macOS distribution

This keeps the feasibility problem focused on compilation first.

## Recommended Dependency/Feature Plan

## `git2`

Prefer one of these:

```toml
# Lowest risk: local repository operations only
git2 = { version = "0.20", default-features = false, features = ["vendored-libgit2"] }

# Networked Git over HTTPS
git2 = { version = "0.20", default-features = false, features = ["https", "vendored-libgit2", "vendored-openssl"] }
```

Avoid enabling `ssh` unless the product really needs it.

## `arboard`

Prefer:

```toml
arboard = { version = "3.6", features = ["wayland-data-control"] }
```

if Linux Wayland support is part of the product promise. Otherwise keep defaults and document the limitation.

## `sherpa-onnx`

Prefer:

```toml
sherpa-onnx = { version = "1.12", default-features = false, features = ["static"] }
```

Use `shared` only if artifact size or library reuse becomes more important than packaging simplicity.

## Final Recommendation

For this spike, the expected feasibility is:

- `x86_64-unknown-linux-gnu`: **Go**
- `aarch64-unknown-linux-gnu`: **Go**
- `aarch64-apple-darwin`: **Go, with CI environment constraint**

The single biggest design change that improves portability is:

1. disable `git2` default features;
2. opt into vendored features deliberately;
3. treat `arboard` as a runtime environment concern on Linux;
4. provide a macOS SDK in CI for the Darwin target.

## Sources

- cargo-zigbuild README: <https://github.com/rust-cross/cargo-zigbuild>
- cargo-zigbuild raw README: <https://raw.githubusercontent.com/rust-cross/cargo-zigbuild/main/README.md>
- `git2` feature flags: <https://docs.rs/crate/git2/latest/features>
- `libgit2-sys` build script: <https://docs.rs/crate/libgit2-sys/latest/source/build.rs>
- `arboard` README: <https://github.com/1Password/arboard/blob/master/README.md>
- `arboard` manifest: <https://raw.githubusercontent.com/1Password/arboard/master/Cargo.toml>
- `sherpa-onnx` feature flags: <https://docs.rs/crate/sherpa-onnx/latest/features>
- `sherpa-onnx-sys` build script: <https://docs.rs/crate/sherpa-onnx-sys/latest/source/build.rs>
- ONNX Runtime compatibility page: <https://onnxruntime.ai/docs/reference/compatibility.html>
- ONNX Runtime v1.23.0 release notes: <https://github.com/microsoft/onnxruntime/releases/tag/v1.23.0>
