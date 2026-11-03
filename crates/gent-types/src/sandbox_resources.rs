use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum SandboxResourceGrant {
    RuntimeReadOnly(PathBuf),
    ConfigurationReadOnly(PathBuf),
    PrivateStateReadWrite(PathBuf),
}

impl SandboxResourceGrant {
    #[must_use]
    pub fn root(&self) -> &Path {
        match self {
            Self::RuntimeReadOnly(root)
            | Self::ConfigurationReadOnly(root)
            | Self::PrivateStateReadWrite(root) => root,
        }
    }

    #[must_use]
    pub const fn writable(&self) -> bool {
        matches!(self, Self::PrivateStateReadWrite(_))
    }
}
