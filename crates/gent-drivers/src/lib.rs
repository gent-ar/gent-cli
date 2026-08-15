//! Public-driver contracts and pure policies; infrastructure owns provider processes.

pub mod buffering;
pub mod discovery;
pub mod interrupt;
pub mod lock;
pub mod normalize;
pub mod process;
pub mod session;
pub mod supervisor;

pub use discovery::PublicProvider;
pub use process::{CapturedStream, ProcessOutput, SystemLauncher, SystemProcess};
pub use session::{DriverSession, OutputLimits, SessionEffect, SessionInput, SessionStatus};
pub use supervisor::{
    LaunchIntent, ProcessLauncher, ProviderLaunch, ProviderProcess, ProviderSupervisor,
    SupervisorError,
};
