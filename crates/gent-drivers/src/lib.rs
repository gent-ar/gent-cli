//! Public-driver contracts and pure policies; infrastructure owns provider processes.

pub mod buffering;
pub mod claude_runner;
pub mod claude_turn_options;
pub mod codex_bootstrap;
pub mod codex_prompt_runner;
pub mod codex_runner;
pub mod codex_session;
pub mod codex_turn;
pub mod conversation_context_input;
pub mod discovery;
pub mod goal_projection;
pub mod installer;
pub mod interrupt;
pub mod launch_spec;
pub mod lock;
pub mod macos_provider_helper;
pub mod message_encoding;
pub mod ndjson;
pub mod node_runtime_lock;
pub mod normalize;
pub mod npm_pack_install;
pub mod output_pump;
pub mod process;
mod process_streams;
pub mod public_protocol;
pub mod run_runner;
pub mod sandboxed_launch;
pub mod session;
mod session_frames;
pub mod supervisor;

pub use discovery::PublicProvider;
pub use macos_provider_helper::{
    MacosHelperDenial, MacosHelperPrepare, MacosProviderHelperClient, MacosProviderHelperError,
    MacosProviderHelperTransport,
};
pub use output_pump::{MAX_OUTPUT_CHUNK_BYTES, OutputPumpError, ProviderOutputPump};
pub use process::{CapturedStream, ProcessOutput, SystemLauncher, SystemProcess};
pub use run_runner::DriverRunRunner;
pub use sandboxed_launch::{
    SandboxedLauncher, SandboxedProviderLaunch, SandboxedProviderLaunchError,
};
pub use session::{DriverSession, OutputLimits, SessionEffect, SessionInput, SessionStatus};
pub use supervisor::{
    LaunchIntent, ProcessLauncher, ProviderLaunch, ProviderProcess, ProviderSupervisor,
    SupervisorError,
};
