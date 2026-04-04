/// Detected capabilities of the current device.
#[derive(Debug, Clone)]
pub struct DeviceCapabilities {
    pub ram_bytes: u64,
    pub cpu_cores: usize,
    pub disk_free_bytes: u64,
    pub is_termux: bool,
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
