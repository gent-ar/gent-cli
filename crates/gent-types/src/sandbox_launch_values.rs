use super::SandboxLaunchContractError;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SandboxCpuSemantics {
    #[default]
    StrictTotal,
    Uncapped,
    MonitoredTotal {
        poll_ms: u16,
        wall_ms: u32,
    },
}
impl SandboxCpuSemantics {
    pub fn validate(self) -> Result<Self, SandboxLaunchContractError> {
        match self {
            Self::StrictTotal | Self::Uncapped => Ok(self),
            Self::MonitoredTotal {
                poll_ms: 1..=100,
                wall_ms: 100..=600_000,
            } => Ok(self),
            Self::MonitoredTotal { .. } => Err(SandboxLaunchContractError::MissingResourceLimit),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxBackendId(String);

impl SandboxBackendId {
    pub fn new(value: String) -> Result<Self, SandboxLaunchContractError> {
        if value.is_empty()
            || value.len() > 80
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-'
            })
        {
            return Err(SandboxLaunchContractError::InvalidBackend);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
