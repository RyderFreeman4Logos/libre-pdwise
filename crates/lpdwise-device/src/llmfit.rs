use std::ffi::OsStr;
use std::path::PathBuf;

use serde::Deserialize;
use sysinfo::System;
use tracing::{debug, warn};

use crate::probe::{Acceleration, DeviceCapabilities, DeviceProber, ProbeError};

const LLMFIT_CLI: &str = "llmfit";
const LLMFIT_INSTALL_SPEC: &str = "cargo:llmfit@latest";

/// Device prober that tries `llmfit system --json` first, then falls back
/// to sysinfo for basic capability detection.
#[derive(Default)]
pub struct LlmfitProber;

/// Partial llmfit JSON output (only fields we need).
#[derive(Debug, Deserialize)]
struct LlmfitOutput {
    #[serde(default)]
    ram_mb: Option<u64>,
    #[serde(default)]
    vram_mb: Option<u64>,
    #[serde(default)]
    cpu_cores: Option<usize>,
    #[serde(default)]
    acceleration: Option<String>,
    #[serde(default)]
    system: Option<LlmfitSystemOutput>,
}

#[derive(Debug, Deserialize)]
struct LlmfitSystemOutput {
    #[serde(default)]
    available_ram_gb: Option<f64>,
    #[serde(default)]
    total_ram_gb: Option<f64>,
    #[serde(default)]
    gpu_vram_gb: Option<f64>,
    #[serde(default)]
    cpu_cores: Option<usize>,
    #[serde(default)]
    backend: Option<String>,
}

impl LlmfitProber {
    pub fn new() -> Self {
        Self
    }

    /// Try probing via `llmfit system --json` CLI.
    fn try_llmfit(&self) -> Option<DeviceCapabilities> {
        let mut failures = Vec::new();

        if let Ok(path) = resolve_mise_binary(LLMFIT_CLI) {
            match run_llmfit_command(path.as_os_str()) {
                Ok(caps) => return Some(caps),
                Err(reason) => failures.push(format!("mise-managed {LLMFIT_CLI}: {reason}")),
            }
        }

        match run_llmfit_command(OsStr::new(LLMFIT_CLI)) {
            Ok(caps) => return Some(caps),
            Err(reason) => failures.push(format!("PATH {LLMFIT_CLI}: {reason}")),
        }

        debug!("llmfit unavailable, attempting to install/activate via mise use -g...");
        match install_with_mise(LLMFIT_INSTALL_SPEC) {
            Ok(()) => match resolve_mise_binary(LLMFIT_CLI) {
                Ok(path) => match run_llmfit_command(path.as_os_str()) {
                    Ok(caps) => return Some(caps),
                    Err(reason) => {
                        failures.push(format!("mise-managed {LLMFIT_CLI} after install: {reason}"))
                    }
                },
                Err(reason) => failures.push(format!(
                    "unable to resolve {LLMFIT_CLI} via mise after install: {reason}"
                )),
            },
            Err(reason) => failures.push(reason),
        }

        warn!(
            attempts = %failures.join(" | "),
            "llmfit unavailable after PATH and mise attempts, falling back to sysinfo"
        );
        None
    }

    fn parse_llmfit_output(stdout: &[u8]) -> Result<DeviceCapabilities, String> {
        let parsed: LlmfitOutput =
            serde_json::from_slice(stdout).map_err(|e| format!("invalid llmfit JSON: {e}"))?;
        let system = parsed.system.as_ref();
        let acceleration = parsed
            .acceleration
            .as_deref()
            .or_else(|| system.and_then(|info| info.backend.as_deref()))
            .map(parse_acceleration)
            .unwrap_or(Acceleration::Cpu);

        Ok(DeviceCapabilities {
            ram_mb: parsed
                .ram_mb
                .or_else(|| {
                    system.and_then(|info| {
                        info.total_ram_gb
                            .or(info.available_ram_gb)
                            .map(gigabytes_to_mb)
                    })
                })
                .unwrap_or(0),
            vram_mb: parsed
                .vram_mb
                .or_else(|| system.and_then(|info| info.gpu_vram_gb.map(gigabytes_to_mb))),
            cpu_cores: parsed
                .cpu_cores
                .or_else(|| system.and_then(|info| info.cpu_cores))
                .unwrap_or(1),
            disk_free_bytes: probe_disk_free(),
            acceleration,
            is_termux: detect_termux(),
        })
    }

    /// Fallback: use sysinfo crate for basic system info.
    fn probe_sysinfo(&self) -> DeviceCapabilities {
        let mut sys = System::new();
        sys.refresh_memory();
        sys.refresh_cpu_all();

        let ram_mb = sys.total_memory() / (1024 * 1024);
        let cpu_cores = sys.cpus().len();

        DeviceCapabilities {
            ram_mb,
            vram_mb: None,
            cpu_cores,
            disk_free_bytes: probe_disk_free(),
            acceleration: Acceleration::Cpu,
            is_termux: detect_termux(),
        }
    }
}

impl DeviceProber for LlmfitProber {
    fn probe(&self) -> Result<DeviceCapabilities, ProbeError> {
        // Try llmfit first for richer info (VRAM, acceleration)
        if let Some(caps) = self.try_llmfit() {
            debug!(?caps, "probed via llmfit");
            return Ok(caps);
        }

        warn!("llmfit probe failed, falling back to sysinfo");
        let caps = self.probe_sysinfo();
        debug!(?caps, "probed via sysinfo");
        Ok(caps)
    }
}

fn parse_acceleration(s: &str) -> Acceleration {
    match s.to_lowercase().as_str() {
        "cuda" | "nvidia" => Acceleration::Cuda,
        "metal" | "mps" => Acceleration::Metal,
        _ => Acceleration::Cpu,
    }
}

fn gigabytes_to_mb(value: f64) -> u64 {
    (value * 1024.0).round().max(0.0) as u64
}

fn run_llmfit_command(program: &OsStr) -> Result<DeviceCapabilities, String> {
    let output = std::process::Command::new(program)
        .args(["system", "--json"])
        .output()
        .map_err(|e| format!("failed to spawn llmfit system --json: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "llmfit system --json exited with {}{}",
            output.status,
            format_command_context(&output.stdout, &output.stderr)
        ));
    }

    LlmfitProber::parse_llmfit_output(&output.stdout)
}

fn resolve_mise_binary(tool: &str) -> Result<PathBuf, String> {
    let output = std::process::Command::new("mise")
        .args(["which", tool])
        .output()
        .map_err(|e| format!("failed to spawn `mise which {tool}`: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "`mise which {tool}` exited with {}{}",
            output.status,
            format_command_context(&output.stdout, &output.stderr)
        ));
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return Err(format!("`mise which {tool}` returned an empty path"));
    }

    Ok(PathBuf::from(path))
}

fn install_with_mise(spec: &str) -> Result<(), String> {
    let output = std::process::Command::new("mise")
        .args(["use", "-g", spec])
        .output()
        .map_err(|e| format!("failed to spawn `mise use -g {spec}`: {e}"))?;

    if output.status.success() {
        return Ok(());
    }

    Err(format!(
        "`mise use -g {spec}` exited with {}{}",
        output.status,
        format_command_context(&output.stdout, &output.stderr)
    ))
}

fn format_command_context(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    let detail = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|line| !line.is_empty());

    match detail {
        Some(line) => format!(": {line}"),
        None => String::new(),
    }
}

fn detect_termux() -> bool {
    std::env::var("TERMUX_VERSION").is_ok()
        || std::path::Path::new("/data/data/com.termux").exists()
}

fn probe_disk_free() -> u64 {
    // Use statvfs on the home directory to get available disk space
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::mem::MaybeUninit;

        let path = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"));
        let c_path = CString::new(path.to_string_lossy().as_bytes())
            .unwrap_or_else(|_| CString::new("/").unwrap());

        let mut stat = MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: statvfs is a POSIX function that writes to the provided
        // buffer. The buffer is properly sized via MaybeUninit<statvfs>.
        let ret = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };
        if ret == 0 {
            // SAFETY: statvfs returned 0 (success), so the buffer is initialized.
            let stat = unsafe { stat.assume_init() };
            return stat.f_bavail.saturating_mul(stat.f_frsize);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_acceleration() {
        assert_eq!(parse_acceleration("cuda"), Acceleration::Cuda);
        assert_eq!(parse_acceleration("NVIDIA"), Acceleration::Cuda);
        assert_eq!(parse_acceleration("metal"), Acceleration::Metal);
        assert_eq!(parse_acceleration("MPS"), Acceleration::Metal);
        assert_eq!(parse_acceleration("cpu"), Acceleration::Cpu);
        assert_eq!(parse_acceleration("unknown"), Acceleration::Cpu);
    }

    #[test]
    fn test_sysinfo_probe_succeeds() {
        let prober = LlmfitProber::new();
        let caps = prober.probe_sysinfo();

        // Should at least detect some RAM and cores
        assert!(caps.ram_mb > 0, "should detect RAM");
        assert!(caps.cpu_cores > 0, "should detect CPU cores");
    }

    #[test]
    fn test_full_probe_succeeds() {
        let prober = LlmfitProber::new();
        let caps = prober.probe().unwrap();

        assert!(caps.ram_mb > 0);
        assert!(caps.cpu_cores > 0);
    }

    #[test]
    fn test_can_run_local_whisper() {
        let caps = DeviceCapabilities {
            ram_mb: 4096,
            vram_mb: None,
            cpu_cores: 4,
            disk_free_bytes: 10_000_000_000,
            acceleration: Acceleration::Cpu,
            is_termux: false,
        };
        assert!(caps.can_run_local_whisper());

        let small = DeviceCapabilities {
            ram_mb: 1024,
            ..caps.clone()
        };
        assert!(!small.can_run_local_whisper());
    }

    #[test]
    fn test_parse_llmfit_nested_system_output() {
        let json = br#"{
          "system": {
            "total_ram_gb": 31.05,
            "gpu_vram_gb": 8.0,
            "cpu_cores": 20,
            "backend": "CUDA"
          }
        }"#;

        let caps = LlmfitProber::parse_llmfit_output(json).unwrap();
        assert_eq!(caps.acceleration, Acceleration::Cuda);
        assert_eq!(caps.cpu_cores, 20);
        assert_eq!(caps.ram_mb, 31_795);
        assert_eq!(caps.vram_mb, Some(8_192));
    }
}
