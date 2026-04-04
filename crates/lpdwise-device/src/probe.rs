use serde::{Deserialize, Serialize};

/// Hardware acceleration type available on the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Acceleration {
    Cpu,
    Cuda,
    Metal,
}

/// Detected capabilities of the current device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCapabilities {
    pub ram_mb: u64,
    pub vram_mb: Option<u64>,
    pub cpu_cores: usize,
    pub disk_free_bytes: u64,
    pub acceleration: Acceleration,
    pub is_termux: bool,
}

impl DeviceCapabilities {
    /// RAM in bytes (convenience for comparisons).
    pub fn ram_bytes(&self) -> u64 {
        self.ram_mb * 1024 * 1024
    }

    /// Whether the device has enough RAM for local whisper inference (~2GB minimum).
    pub fn can_run_local_whisper(&self) -> bool {
        self.ram_mb >= 2048
    }
}

/// Abstraction for probing device capabilities at runtime.
pub trait DeviceProber {
    /// Detect and return the current device's capabilities.
    fn probe(&self) -> Result<DeviceCapabilities, ProbeError>;
}

/// Errors from device probing.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("failed to query system info: {0}")]
    SystemQuery(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
