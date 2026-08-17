//! Terminal requests for the secret-free provider-auth protocol extension.

use clap::{Subcommand, ValueEnum};
use gent_protocol::{
    PROVIDER_AUTH_CAPABILITY, ProviderAuthFrame, WireFrame, read_json_frame,
    write_provider_auth_frame,
};
use gent_types::ProviderAuthProvider;
use serde_json::Value;
use std::path::PathBuf;

use crate::local_ipc::connect_and_negotiate;

/// Terminal-only provider authentication actions; credentials never enter this boundary.
#[derive(Debug, Subcommand)]
pub(crate) enum ProviderAuthCommand {
    /// Read the daemon-owned authentication state for one public provider.
    Status { provider: ProviderArgument },
    /// Request a typed `askTool` login choice; this does not start a provider process.
    Login { provider: ProviderArgument },
}

/// Public providers supported by the secret-free authentication contract.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ProviderArgument {
    Claude,
    Codex,
}

/// Exchanges a request with gentd and returns a displayable, secret-free response.
pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: ProviderAuthCommand,
) -> Result<ProviderAuthFrame, Box<dyn std::error::Error>> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let request = match command {
        ProviderAuthCommand::Status { provider } => ProviderAuthFrame::StatusRequest {
            request_id,
            provider: provider.into(),
        },
        ProviderAuthCommand::Login { provider } => ProviderAuthFrame::LoginRequest {
            request_id,
            provider: provider.into(),
        },
    };
    exchange(data_dir, no_autostart, request).await
}

async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: ProviderAuthFrame,
) -> Result<ProviderAuthFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == PROVIDER_AUTH_CAPABILITY)
    {
        return Err(
            "provider authentication is unavailable while gentd runs in observer mode; no provider login was started"
                .into(),
        );
    }
    write_provider_auth_frame(&mut stream, &request).await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(frame) = serde_json::from_value(raw.clone()) {
        return Ok(frame);
    }
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw) {
        return Err(message.into());
    }
    Err("daemon did not return a provider authentication frame".into())
}

impl From<ProviderArgument> for ProviderAuthProvider {
    fn from(value: ProviderArgument) -> Self {
        match value {
            ProviderArgument::Claude => Self::Claude,
            ProviderArgument::Codex => Self::Codex,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use clap::Parser;
    use gent_protocol::{
        Negotiated, PROVIDER_AUTH_CAPABILITY, ProviderAuthFrame, WireFrame, read_frame,
        read_json_frame, write_frame,
    };
    use gent_types::{CapabilitySet, PROTOCOL_MAX, ProviderAuthProvider};
    use tokio::net::UnixListener;

    use super::{ProviderAuthCommand, execute};
    use crate::{Args, CommandLine};

    #[test]
    fn parses_public_provider_status_and_login_commands() {
        let status = Args::try_parse_from(["gent", "auth", "status", "claude"]).unwrap();
        assert!(matches!(
            status.command,
            Some(CommandLine::Auth {
                action: ProviderAuthCommand::Status { .. }
            })
        ));
        let login = Args::try_parse_from(["gent", "auth", "login", "codex"]).unwrap();
        assert!(matches!(
            login.command,
            Some(CommandLine::Auth {
                action: ProviderAuthCommand::Login { .. }
            })
        ));
    }

    #[tokio::test]
    async fn observer_capability_reports_a_controlled_no_login_message() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_frame(&mut stream).await.unwrap();
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet::default(),
                }),
            )
            .await
            .unwrap();
        });
        let error = execute(
            Some(directory.path().into()),
            true,
            ProviderAuthCommand::Login {
                provider: super::ProviderArgument::Claude,
            },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("observer mode"));
    }

    #[tokio::test]
    async fn login_sends_a_provider_only_secret_free_login_request() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                WireFrame::Hello(hello)
                    if hello.capabilities.0.iter().any(|item| item == PROVIDER_AUTH_CAPABILITY)
            ));
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![PROVIDER_AUTH_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            assert!(matches!(
                read_json_frame::<_, ProviderAuthFrame>(&mut stream)
                    .await
                    .unwrap(),
                ProviderAuthFrame::LoginRequest {
                    provider: ProviderAuthProvider::Codex,
                    ..
                }
            ));
            write_frame(
                &mut stream,
                &WireFrame::Error {
                    code: "providerAuthDisabled".into(),
                    message: "provider authentication is disabled".into(),
                },
            )
            .await
            .unwrap();
        });
        let error = execute(
            Some(directory.path().into()),
            true,
            ProviderAuthCommand::Login {
                provider: super::ProviderArgument::Codex,
            },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("disabled"));
    }
}
