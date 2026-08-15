//! Public-driver contracts and pure policies; infrastructure owns provider processes.

pub mod buffering;
pub mod discovery;
pub mod installer;
pub mod interrupt;
pub mod launch_spec;
pub mod lock;
pub mod message_encoding;
pub mod ndjson;
pub mod normalize;
pub mod output_pump;
pub mod process;
mod process_streams;
pub mod public_protocol;
pub mod run_runner;
pub mod session;
mod session_frames;
pub mod supervisor;

pub use discovery::PublicProvider;
pub use output_pump::{MAX_OUTPUT_CHUNK_BYTES, OutputPumpError, ProviderOutputPump};
pub use process::{CapturedStream, ProcessOutput, SystemLauncher, SystemProcess};
pub use run_runner::DriverRunRunner;
pub use session::{DriverSession, OutputLimits, SessionEffect, SessionInput, SessionStatus};
pub use supervisor::{
    LaunchIntent, ProcessLauncher, ProviderLaunch, ProviderProcess, ProviderSupervisor,
    SupervisorError,
};
