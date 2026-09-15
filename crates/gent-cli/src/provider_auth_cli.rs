//! Terminal requests for the secret-free provider-auth protocol extension.

use clap::{Subcommand, ValueEnum};
use gent_protocol::{
    PROVIDER_AUTH_CAPABILITY, ProviderAuthFrame, read_json_frame, write_json_frame,
};
use gent_types::{
    ProviderAuthLifecycle, ProviderAuthMethod, ProviderAuthMethodSelection, ProviderAuthProvider,
};
use serde_json::Value;
use std::{future::Future, path::PathBuf, time::Duration};

const SETTLE_POLL: Duration = Duration::from_millis(500);
const SETTLE_LIMIT: Duration = Duration::from_secs(600);

/// Terminal-only provider authentication actions.
#[derive(Debug, Subcommand)]
pub(crate) enum ProviderAuthCommand {
    #[command(about = "Read provider authentication state from Gentd")]
    Status {
        #[arg(help = "Provider to check")]
        provider: ProviderArgument,
        #[arg(long, help = "Wait until Gentd finishes checking the provider account")]
        wait: bool,
    },
    #[command(about = "Start the provider-owned browser login flow through Gentd")]
    Login {
        #[arg(help = "Provider to sign in to")]
        provider: ProviderArgument,
    },
}

/// Public providers supported by the secret-free authentication contract.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ProviderArgument {
    Claude,
    Codex,
}

pub(crate) async fn login_interactive(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    provider: ProviderArgument,
) -> Result<String, String> {
    let provider = provider.into();
    let request_id = uuid::Uuid::new_v4().to_string();
    let response = exchange(
        data_dir.clone(),
        no_autostart,
        ProviderAuthFrame::LoginRequest {
            request_id,
            provider,
        },
    )
    .await?;
    match response {
        ProviderAuthFrame::AskTool { challenge, .. }
            if challenge
                .methods
                .contains(&ProviderAuthMethod::AccountBrowser) =>
        {
            let selected = exchange(
                data_dir,
                no_autostart,
                ProviderAuthFrame::SelectMethod {
                    request_id: uuid::Uuid::new_v4().to_string(),
                    selection: ProviderAuthMethodSelection {
                        challenge_id: challenge.challenge_id,
                        method: ProviderAuthMethod::AccountBrowser,
                    },
                },
            )
            .await?;
            login_notice(provider, selected)
        }
        frame => login_notice(provider, frame),
    }
}

pub(crate) async fn status(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    provider: ProviderArgument,
    wait: bool,
) -> Result<ProviderAuthFrame, String> {
    let read = || {
        exchange(
            data_dir.clone(),
            no_autostart,
            ProviderAuthFrame::StatusRequest {
                request_id: uuid::Uuid::new_v4().to_string(),
                provider: provider.into(),
            },
        )
    };
    if wait {
        settled(read, SETTLE_POLL, SETTLE_LIMIT).await
    } else {
        read().await
    }
}

async fn settled<F, R>(
    read: F,
    poll: Duration,
    limit: Duration,
) -> Result<ProviderAuthFrame, String>
where
    F: Fn() -> R,
    R: Future<Output = Result<ProviderAuthFrame, String>>,
{
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        let frame = read().await?;
        let checking = matches!(
            &frame,
            ProviderAuthFrame::Status { status, .. }
                if status.lifecycle == ProviderAuthLifecycle::Checking
        );
        if !checking {
            return Ok(frame);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("gentd is still checking the provider account".into());
        }
        tokio::time::sleep(poll).await;
    }
}

impl From<ProviderArgument> for ProviderAuthProvider {
    fn from(value: ProviderArgument) -> Self {
        match value {
            ProviderArgument::Claude => Self::Claude,
            ProviderArgument::Codex => Self::Codex,
        }
    }
}

async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: ProviderAuthFrame,
) -> Result<ProviderAuthFrame, String> {
    let (mut stream, capabilities) =
        crate::local_ipc::connect_and_negotiate(data_dir, no_autostart)
            .await
            .map_err(|error| error.to_string())?;
    if !capabilities
        .0
        .iter()
        .any(|value| value == PROVIDER_AUTH_CAPABILITY)
    {
        return Err("gentd does not support provider authentication; upgrade gentd".into());
    }
    write_json_frame(&mut stream, &request)
        .await
        .map_err(|error| error.to_string())?;
    let raw: Value = read_json_frame(&mut stream)
        .await
        .map_err(|error| error.to_string())?;
    if let Some(error) = crate::cli_error::CliError::from_reply(&raw) {
        return Err(error.to_string());
    }
    serde_json::from_value(raw)
        .map_err(|_| "gentd returned an invalid provider authentication frame".into())
}

fn login_notice(
    provider: ProviderAuthProvider,
    frame: ProviderAuthFrame,
) -> Result<String, String> {
    let label = match provider {
        ProviderAuthProvider::Claude => "Claude",
        ProviderAuthProvider::Codex => "Codex",
    };
    let status = match frame {
        ProviderAuthFrame::Status { status, .. }
        | ProviderAuthFrame::SelectionAccepted { status, .. } => status,
        _ => return Err("gentd returned an invalid provider authentication response".into()),
    };
    match status.lifecycle {
        ProviderAuthLifecycle::NotInstalled => Err(format!(
            "{label} is not installed in the managed runtime. Start a {label} conversation first."
        )),
        ProviderAuthLifecycle::Authenticated => Ok(format!(
            "{label} is authenticated. You can continue this conversation."
        )),
        ProviderAuthLifecycle::Verifying => Ok(format!(
            "{label} sign-in is active in your browser. Gentd will verify it when you return."
        )),
        ProviderAuthLifecycle::Checking => Err(format!(
            "Gentd is still checking {label}. Run `gent auth status {} --wait`, then try again.",
            label.to_lowercase()
        )),
        ProviderAuthLifecycle::Cancelled => Err(format!("{label} sign-in was cancelled.")),
        ProviderAuthLifecycle::Expired | ProviderAuthLifecycle::TimedOut => {
            Err(format!("{label} sign-in expired. Run /login to try again."))
        }
        ProviderAuthLifecycle::ProviderChanged => Err(format!(
            "{label} changed during sign-in. Run /login to try again."
        )),
        ProviderAuthLifecycle::Failed => Err(format!("{label} sign-in could not be started.")),
        _ => Err(format!("{label} is not authenticated.")),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use gent_protocol::ProviderAuthFrame;
    use gent_types::{ProviderAuthLifecycle, ProviderAuthProvider, ProviderAuthStatus};

    use super::{ProviderAuthCommand, settled};
    use crate::{Args, CommandLine};

    fn status(lifecycle: ProviderAuthLifecycle) -> ProviderAuthFrame {
        ProviderAuthFrame::Status {
            request_id: "status".into(),
            status: ProviderAuthStatus {
                provider: ProviderAuthProvider::Claude,
                binary_lock: None,
                lifecycle,
                selected_method: None,
                expires_at_unix_seconds: None,
            },
        }
    }

    #[tokio::test]
    async fn waiting_status_rereads_checking_until_gentd_settles_the_account() {
        let reads = AtomicUsize::new(0);
        let frame = settled(
            || async {
                Ok(status(if reads.fetch_add(1, Ordering::SeqCst) < 2 {
                    ProviderAuthLifecycle::Checking
                } else {
                    ProviderAuthLifecycle::Authenticated
                }))
            },
            Duration::from_millis(1),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(frame, status(ProviderAuthLifecycle::Authenticated));
        assert_eq!(reads.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn waiting_status_is_bounded_while_gentd_keeps_checking() {
        let error = settled(
            || async { Ok(status(ProviderAuthLifecycle::Checking)) },
            Duration::from_millis(1),
            Duration::from_millis(20),
        )
        .await
        .unwrap_err();
        assert!(error.contains("still checking"), "{error}");
    }

    #[test]
    fn checking_serializes_as_a_plain_status_for_scripts() {
        let encoded = serde_json::to_value(status(ProviderAuthLifecycle::Checking)).unwrap();
        assert_eq!(encoded["body"]["status"]["lifecycle"], "checking");
    }

    #[test]
    fn parses_status_wait_flag() {
        let status = Args::try_parse_from(["gent", "auth", "status", "codex", "--wait"]).unwrap();
        assert!(matches!(
            status.command,
            Some(CommandLine::Auth {
                action: ProviderAuthCommand::Status { wait: true, .. }
            })
        ));
    }

    #[test]
    fn parses_public_provider_login_command() {
        let login = Args::try_parse_from(["gent", "auth", "login", "codex"]).unwrap();
        assert!(matches!(
            login.command,
            Some(CommandLine::Auth {
                action: ProviderAuthCommand::Login { .. }
            })
        ));
    }

    #[test]
    fn parses_public_provider_status_command() {
        let status = Args::try_parse_from(["gent", "auth", "status", "claude"]).unwrap();
        assert!(matches!(
            status.command,
            Some(CommandLine::Auth {
                action: ProviderAuthCommand::Status { .. }
            })
        ));
    }
}
