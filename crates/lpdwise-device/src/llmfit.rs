use serde::Deserialize;
use sysinfo::System;
use tracing::{debug, warn};

use crate::probe::{Acceleration, DeviceCapabilities, DeviceProber, ProbeError};

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
}

impl LlmfitProber {
    pub fn new() -> Self {
        Self
    }

    /// Try probing via `llmfit system --json` CLI.
    fn try_llmfit(&self) -> Option<DeviceCapabilities> {
        let run_cmd = || {
            std::process::Command::new("llmfit")
                .args(["system", "--json"])
                .output()
        };

        let mut output = run_cmd();

        if output.is_err() {
            // Try to auto-install via mise if not found
            debug!("llmfit not found, attempting to install via mise...");
            let install_status = std::process::Command::new("mise")
                .args(["use", "-g", "cargo:llmfit@latest"])
                .status();

            if let Ok(status) = install_status {
                if status.success() {
                    debug!("llmfit successfully installed via mise");
                    // Try running again after installation
                    output = run_cmd();
                    
                    // If llmfit is still not in PATH (shims not configured), try running it via mise exec
                    if output.is_err() {
                        output = std::process::Command::new("mise")
                            .args(["exec", "--", "llmfit", "system", "--json"])
                            .output();
                    }
                } else {
                    warn!("failed to install llmfit via mise");
                }
            }
        }

        let output = output.ok()?;

        if !output.status.success() {
            return None;
        }

        let parsed: LlmfitOutput = serde_json::from_slice(&output.stdout).ok()?;

        let acceleration = parsed
            .acceleration
            .as_deref()
            .map(parse_acceleration)
            .unwrap_or(Acceleration::Cpu);

        Some(DeviceCapabilities {
            ram_mb: parsed.ram_mb.unwrap_or(0),
            vram_mb: parsed.vram_mb,
            cpu_cores: parsed.cpu_cores.unwrap_or(1),
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

        warn!("llmfit not available, falling back to sysinfo");
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
}
