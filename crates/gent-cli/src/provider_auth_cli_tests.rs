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
            challenge_id: None,
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
