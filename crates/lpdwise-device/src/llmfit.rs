use crate::probe::{DeviceCapabilities, DeviceProber, ProbeError};

/// Device prober that uses platform-native APIs and heuristics.
#[derive(Default)]
pub struct LlmfitProber;

impl LlmfitProber {
    pub fn new() -> Self {
        Self
    }
}

impl DeviceProber for LlmfitProber {
    fn probe(&self) -> Result<DeviceCapabilities, ProbeError> {
        todo!("implement device capability probing")
    }
}
