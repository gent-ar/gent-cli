//! Terminal requests for the secret-free provider-auth protocol extension.

use clap::{Subcommand, ValueEnum};
use gent_protocol::{
    PROVIDER_AUTH_CAPABILITY, PROVIDER_AUTH_LOGOUT_CAPABILITY, ProviderAuthFrame, write_json_frame,
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
        #[arg(long, help = "Sign out first, then sign in again with another account")]
        again: bool,
    },
    #[command(about = "Sign out of the provider account through Gentd")]
    Logout {
        #[arg(help = "Provider to sign out of")]
        provider: ProviderArgument,
    },
}

/// Public providers supported by the secret-free authentication contract.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ProviderArgument {
    Claude,
    Codex,
}

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: ProviderAuthCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        ProviderAuthCommand::Status { provider, wait } => {
            crate::command_execution::print(status(data_dir, no_autostart, provider, wait).await?)?;
        }
        ProviderAuthCommand::Login { provider, again } => {
            println!("{}", login(data_dir, no_autostart, provider, again).await?);
        }
        ProviderAuthCommand::Logout { provider } => {
            println!("{}", logout(data_dir, no_autostart, provider).await?);
        }
    }
    Ok(())
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

pub(crate) async fn login(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    provider: ProviderArgument,
    again: bool,
) -> Result<String, String> {
    if again {
        logout(data_dir.clone(), no_autostart, provider).await?;
    }
    login_interactive(data_dir, no_autostart, provider).await
}

pub(crate) async fn logout(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    provider: ProviderArgument,
) -> Result<String, String> {
    let provider = provider.into();
    let frame = exchange(
        data_dir,
        no_autostart,
        ProviderAuthFrame::LogoutRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            provider,
        },
    )
    .await?;
    logout_notice(provider, &frame)
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
    let required: &[&str] = if matches!(request, ProviderAuthFrame::LogoutRequest { .. }) {
        &[PROVIDER_AUTH_CAPABILITY, PROVIDER_AUTH_LOGOUT_CAPABILITY]
    } else {
        &[PROVIDER_AUTH_CAPABILITY]
    };
    if let Some(missing) = required
        .iter()
        .find(|required| !capabilities.0.iter().any(|value| value == *required))
    {
        return Err(format!("gentd does not support {missing}; upgrade gentd"));
    }
    write_json_frame(&mut stream, &request)
        .await
        .map_err(|error| error.to_string())?;
    let raw: Value = crate::local_ipc::read_reply(&mut stream)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::from_value(raw)
        .map_err(|_| "gentd returned an invalid provider authentication frame".into())
}

fn login_notice(
    provider: ProviderAuthProvider,
    frame: ProviderAuthFrame,
) -> Result<String, String> {
    let label = label(provider);
    let (ProviderAuthFrame::Status { status, .. }
    | ProviderAuthFrame::SelectionAccepted { status, .. }) = frame
    else {
        return Err("gentd returned an invalid provider authentication response".into());
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

const fn label(provider: ProviderAuthProvider) -> &'static str {
    match provider {
        ProviderAuthProvider::Claude => "Claude",
        ProviderAuthProvider::Codex => "Codex",
    }
}

fn logout_notice(
    provider: ProviderAuthProvider,
    frame: &ProviderAuthFrame,
) -> Result<String, String> {
    let label = label(provider);
    let ProviderAuthFrame::Status { status, .. } = frame else {
        return Err("gentd returned an invalid provider authentication response".into());
    };
    match status.lifecycle {
        ProviderAuthLifecycle::Unauthenticated => Ok(format!("Signed out of {label}.")),
        ProviderAuthLifecycle::NotInstalled => Err(format!(
            "{label} is not installed in the managed runtime. Start a {label} conversation first."
        )),
        ProviderAuthLifecycle::Authenticated => {
            Err(format!("{label} is still signed in after sign-out."))
        }
        _ => Err(format!("{label} sign-out could not be confirmed.")),
    }
}

#[cfg(test)]
#[path = "provider_auth_cli_tests.rs"]
mod tests;
