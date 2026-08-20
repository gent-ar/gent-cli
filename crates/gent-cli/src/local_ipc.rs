//! Local transport selection; command composition remains in `main`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_INTENTS_CAPABILITY,
    AGENT_CHAT_TRANSCRIPT_CAPABILITY, AGENT_CHAT_TURN_FOLLOW_CAPABILITY,
    CONVERSATION_ACTIVITY_CAPABILITY, CONVERSATION_INDEX_CAPABILITY,
    CONVERSATION_STATUS_CAPABILITY, CONVERSATION_TIMELINE_CAPABILITY, EVENT_STREAM_CAPABILITY,
    GOAL_CAPABILITY, Hello, ORCHESTRATION_CAPABILITY, PERMISSION_POLICY_CAPABILITY,
    PROMPT_PROVIDER_PROVISION_CAPABILITY, PROVIDER_AUTH_CAPABILITY, PROVIDER_READINESS_CAPABILITY,
    REVIEWED_PLAN_CAPABILITY, RUNTIME_MAINTENANCE_CAPABILITY, RUNTIME_UPDATE_CHECK_CAPABILITY,
    WireFrame, read_frame, write_frame,
};
use gent_types::{CapabilitySet, PROTOCOL_MAX, PROTOCOL_MIN};

#[cfg(unix)]
use gent_protocol::CONVERSATION_CONTENT_CAPABILITY;
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
}

#[must_use]
pub(crate) fn client_capabilities() -> CapabilitySet {
    #[cfg(unix)]
    let mut capabilities = vec![
        AGENT_CHAT_INTENTS_CAPABILITY.into(),
        AGENT_CHAT_CONVERSATIONS_CAPABILITY.into(),
        AGENT_CHAT_TRANSCRIPT_CAPABILITY.into(),
        AGENT_CHAT_TURN_FOLLOW_CAPABILITY.into(),
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
        "events".into(),
        "host-epoch".into(),
        "receipts".into(),
    ];
    #[cfg(not(unix))]
    let capabilities = vec![
        AGENT_CHAT_INTENTS_CAPABILITY.into(),
        AGENT_CHAT_CONVERSATIONS_CAPABILITY.into(),
        AGENT_CHAT_TRANSCRIPT_CAPABILITY.into(),
        AGENT_CHAT_TURN_FOLLOW_CAPABILITY.into(),
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
        "events".into(),
        "host-epoch".into(),
        "receipts".into(),
    ];
    #[cfg(unix)]
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
        Err(error) if pipe_is_busy(&error) => return wait_for_connection(data_dir).await,
        Err(_) => {}
    }
    if no_autostart {
        return Err("gentd is unavailable and --no-autostart was requested".into());
    }
    let daemon = std::env::var_os("GENTD_BIN").map_or_else(default_daemon_binary, PathBuf::from);
    let mut command = tokio::process::Command::new(daemon);
    command
        .args(daemon_arguments(data_dir)?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    wait_for_connection(data_dir).await
}

fn daemon_arguments(data_dir: &Path) -> Result<Vec<OsString>, Box<dyn std::error::Error>> {
    daemon_arguments_from(
        data_dir,
        std::env::var_os("GENT_ORDINARY_AUTHORITY"),
        std::env::var_os("GENT_ORDINARY_AUTHORITY_RELEASE"),
        std::env::var_os("GENT_ORDINARY_AUTHORITY_KEY"),
    )
}

fn daemon_arguments_from(
    data_dir: &Path,
    enabled: Option<OsString>,
    release: Option<OsString>,
    keys: Option<OsString>,
) -> Result<Vec<OsString>, Box<dyn std::error::Error>> {
    let mut arguments = vec![OsString::from("--data-dir"), data_dir.as_os_str().into()];
    if ordinary_authority_requested(enabled, release, keys)? {
        arguments.push(OsString::from("--ordinary-authority"));
    } else {
        // A standalone Gent client must at least own its durable conversations. Provider process
        // authority remains separately composed by gentd's ordinary/local provider profiles.
        arguments.push(OsString::from("--agent-chat-authority"));
    }
    Ok(arguments)
}

fn ordinary_authority_requested(
    enabled: Option<OsString>,
    release: Option<OsString>,
    keys: Option<OsString>,
) -> Result<bool, String> {
    let configured = release.is_some() || keys.is_some();
    let Some(enabled) = enabled else {
        return configured
            .then_some(Err(
                "ordinary authority settings require GENT_ORDINARY_AUTHORITY=1".into(),
            ))
            .unwrap_or(Ok(false));
    };
    match enabled.to_string_lossy().as_ref() {
        "1" | "true" => {
            if release.is_none() {
                return Err("ordinary authority requires GENT_ORDINARY_AUTHORITY_RELEASE".into());
            }
            if keys.is_none() {
                return Err("ordinary authority requires GENT_ORDINARY_AUTHORITY_KEY".into());
            }
            Ok(true)
        }
        "0" | "false" => configured
            .then_some(Err(
                "ordinary authority release settings require GENT_ORDINARY_AUTHORITY=1".into(),
            ))
            .unwrap_or(Ok(false)),
        _ => Err("GENT_ORDINARY_AUTHORITY must be 1, 0, true, or false".into()),
    }
}

async fn wait_for_connection(data_dir: &Path) -> Result<LocalStream, Box<dyn std::error::Error>> {
    for _ in 0..40 {
        if let Ok(stream) = connect(data_dir).await {
            return Ok(stream);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Err("gentd did not become ready; set GENTD_BIN to the daemon executable".into())
}

#[cfg(unix)]
async fn connect(data_dir: &Path) -> Result<LocalStream, std::io::Error> {
    UnixStream::connect(data_dir.join("gentd.sock")).await
}

#[cfg(windows)]
#[allow(clippy::unused_async)] // Keeps the shared call site transport-agnostic.
async fn connect(data_dir: &Path) -> Result<LocalStream, std::io::Error> {
    ClientOptions::new().open(pipe_name(data_dir))
}

#[cfg(windows)]
fn pipe_is_busy(error: &std::io::Error) -> bool {
    // ERROR_PIPE_BUSY from the Win32 API. Tokio's client docs prescribe retrying it.
    error.raw_os_error() == Some(231)
}

fn default_daemon_binary() -> PathBuf {
    let name = if cfg!(windows) { "gentd.exe" } else { "gentd" };
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(name)))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

pub(crate) fn default_data_dir() -> PathBuf {
    directories::BaseDirs::new().map_or_else(
        || PathBuf::from(".gentd"),
        |directories| directories.home_dir().join(".gentd"),
    )
}

#[cfg(windows)]
fn pipe_name(data_dir: &Path) -> String {
    format!(r"\\.\pipe\gentd-{:016x}", endpoint_hash(data_dir))
}

#[cfg(windows)]
fn endpoint_hash(data_dir: &Path) -> u64 {
    data_dir
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

#[cfg(all(test, unix))]
#[path = "local_ipc_tests.rs"]
mod tests;
