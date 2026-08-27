//! Local transport selection; command composition remains in `main`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_INTENTS_CAPABILITY,
    AGENT_CHAT_PERMISSIONS_CAPABILITY, AGENT_CHAT_SESSIONS_CAPABILITY,
    AGENT_CHAT_TRANSCRIPT_CAPABILITY, AGENT_CHAT_TURN_FOLLOW_CAPABILITY, ATTACHMENTS_CAPABILITY,
    CONVERSATION_ACTIVITY_CAPABILITY, CONVERSATION_CONTENT_CAPABILITY,
    CONVERSATION_INDEX_CAPABILITY, CONVERSATION_STATUS_CAPABILITY,
    CONVERSATION_TIMELINE_CAPABILITY, EVENT_STREAM_CAPABILITY, GOAL_CAPABILITY, Hello,
    LOCAL_MODELS_CAPABILITY, ORCHESTRATION_CAPABILITY, PERMISSION_POLICY_CAPABILITY,
    PROMPT_PROVIDER_PROVISION_CAPABILITY, PROVIDER_AUTH_CAPABILITY, PROVIDER_READINESS_CAPABILITY,
    REVIEWED_PLAN_CAPABILITY, RUNTIME_MAINTENANCE_CAPABILITY, RUNTIME_UPDATE_CHECK_CAPABILITY,
    WireFrame, read_frame, write_frame,
};
use gent_types::{CapabilitySet, PROTOCOL_MAX, PROTOCOL_MIN};
use gent_types::{default_data_dir as resolve_default_data_dir, resolve_sibling_binary};
#[cfg(unix)]
use gent_types::local_socket_path;
#[cfg(windows)]
use gent_types::windows_pipe_name;

const NEGOTIATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

pub(crate) async fn request(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    frame: WireFrame,
) -> Result<WireFrame, Box<dyn std::error::Error>> {
    let (mut stream, _) = connect_and_negotiate(data_dir, no_autostart).await?;
    write_frame(&mut stream, &frame).await?;
    let response = read_frame(&mut stream).await?;
    if let WireFrame::Error { message, .. } = &response {
        return Err(message.clone().into());
    }
    Ok(response)
}

/// Opens local IPC and requires the mandatory protocol handshake before any extension frame.
pub(crate) async fn connect_and_negotiate(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Result<(LocalStream, CapabilitySet), Box<dyn std::error::Error>> {
    let data_dir = data_dir.unwrap_or_else(default_data_dir);
    let mut stream = connect_or_start(&data_dir, no_autostart).await?;
    tokio::time::timeout(NEGOTIATION_TIMEOUT, async {
        write_frame(
            &mut stream,
            &WireFrame::Hello(Hello {
                protocol_min: PROTOCOL_MIN,
                protocol_max: PROTOCOL_MAX,
                capabilities: client_capabilities(),
            }),
        )
        .await?;
        match read_frame(&mut stream).await? {
            WireFrame::Negotiated(answer) => Ok((stream, answer.capabilities)),
            WireFrame::Error { message, .. } => Err(message.into()),
            _ => Err("daemon did not negotiate protocol".into()),
        }
    })
    .await
    .map_err(|_| "gentd did not negotiate within 3 seconds")?
}

#[must_use]
pub(crate) fn client_capabilities() -> CapabilitySet {
    let mut capabilities = vec![
        AGENT_CHAT_INTENTS_CAPABILITY.into(),
        AGENT_CHAT_PERMISSIONS_CAPABILITY.into(),
        AGENT_CHAT_CONVERSATIONS_CAPABILITY.into(),
        AGENT_CHAT_TRANSCRIPT_CAPABILITY.into(),
        AGENT_CHAT_TURN_FOLLOW_CAPABILITY.into(),
        AGENT_CHAT_SESSIONS_CAPABILITY.into(),
        ATTACHMENTS_CAPABILITY.into(),
        CONVERSATION_ACTIVITY_CAPABILITY.into(),
        CONVERSATION_INDEX_CAPABILITY.into(),
        CONVERSATION_STATUS_CAPABILITY.into(),
        CONVERSATION_TIMELINE_CAPABILITY.into(),
        RUNTIME_MAINTENANCE_CAPABILITY.into(),
        RUNTIME_UPDATE_CHECK_CAPABILITY.into(),
        PERMISSION_POLICY_CAPABILITY.into(),
        PROVIDER_AUTH_CAPABILITY.into(),
        PROVIDER_READINESS_CAPABILITY.into(),
        PROMPT_PROVIDER_PROVISION_CAPABILITY.into(),
        REVIEWED_PLAN_CAPABILITY.into(),
        "decisions".into(),
        EVENT_STREAM_CAPABILITY.into(),
        GOAL_CAPABILITY.into(),
        ORCHESTRATION_CAPABILITY.into(),
        LOCAL_MODELS_CAPABILITY.into(),
        "events".into(),
        "host-epoch".into(),
        "receipts".into(),
    ];
    capabilities.push(CONVERSATION_CONTENT_CAPABILITY.into());
    CapabilitySet(capabilities)
}

#[cfg(unix)]
pub(crate) type LocalStream = UnixStream;
#[cfg(windows)]
pub(crate) type LocalStream = NamedPipeClient;

pub(crate) async fn connect_or_start(
    data_dir: &Path,
    no_autostart: bool,
) -> Result<LocalStream, Box<dyn std::error::Error>> {
    match connect(data_dir).await {
        Ok(stream) => return Ok(stream),
        #[cfg(windows)]
        Err(error) if pipe_is_busy(&error) => {
            return wait_for_connection_until(data_dir, None).await;
        }
        Err(_) => {}
    }
    if no_autostart {
        return Err("gentd is unavailable and --no-autostart was requested".into());
    }
    let daemon = std::env::var_os("GENTD_BIN").map_or_else(default_daemon_binary, PathBuf::from);
    let daemon_display = daemon.display().to_string();
    let mut command = tokio::process::Command::new(daemon);
    command
        .args(daemon_arguments(data_dir))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!(
                "gentd was not found at {daemon_display}; build it with `cargo build -p gentd` or set GENTD_BIN to its executable"
            )
        } else {
            format!("could not start gentd at {daemon_display}: {error}")
        }
    })?;
    wait_for_spawned_connection(data_dir, &mut child).await
}

fn daemon_arguments(data_dir: &Path) -> Vec<OsString> {
    daemon_arguments_from(data_dir)
}

fn daemon_arguments_from(data_dir: &Path) -> Vec<OsString> {
    vec![
        OsString::from("--data-dir"),
        data_dir.as_os_str().into(),
        OsString::from("--standalone-authority"),
    ]
}

async fn wait_for_spawned_connection(
    data_dir: &Path,
    child: &mut tokio::process::Child,
) -> Result<LocalStream, Box<dyn std::error::Error>> {
    wait_for_connection_until(data_dir, Some(child)).await
}

async fn wait_for_connection_until(
    data_dir: &Path,
    mut child: Option<&mut tokio::process::Child>,
) -> Result<LocalStream, Box<dyn std::error::Error>> {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            if let Ok(stream) = connect(data_dir).await {
                return Ok(stream);
            }
            if let Some(child) = child.as_deref_mut() {
                if let Some(status) = child.try_wait()? {
                    return Err(format!("gentd exited before becoming ready ({status})").into());
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| -> Box<dyn std::error::Error> {
        "gentd did not become ready within 60 seconds".into()
    })?
}

#[cfg(unix)]
async fn connect(data_dir: &Path) -> Result<LocalStream, std::io::Error> {
    UnixStream::connect(local_socket_path(data_dir)).await
}

#[cfg(windows)]
async fn connect(data_dir: &Path) -> Result<LocalStream, std::io::Error> {
    ClientOptions::new().open(windows_pipe_name(data_dir))
}

#[cfg(windows)]
fn pipe_is_busy(error: &std::io::Error) -> bool {
    // ERROR_PIPE_BUSY from the Win32 API. Tokio's client docs prescribe retrying it.
    error.raw_os_error() == Some(231)
}

pub(crate) fn default_daemon_binary() -> PathBuf {
    let name = if cfg!(windows) { "gentd.exe" } else { "gentd" };
    resolve_sibling_binary(name)
}

pub(crate) fn default_data_dir() -> PathBuf {
    resolve_default_data_dir()
}

#[cfg(all(test, unix))]
#[path = "local_ipc_tests.rs"]
mod tests;
