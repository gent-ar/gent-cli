use clap::Parser;
use gent_protocol::{CONVERSATION_ACTIVITY_CAPABILITY, REVIEWED_PLAN_CAPABILITY};
use gent_runtime::catalog::declared_capabilities_with_profiles;
#[cfg(unix)]
use std::time::Duration;

use super::{Args, claurst_runtime_config, standalone_capability_profile, validate};

fn args(extra: &[&str]) -> Args {
    let mut values = vec![
        "gentd",
        "--standalone-authority",
        "--standalone-claude-executable",
        "/bin/sh",
        "--standalone-codex-executable",
        "/bin/sh",
    ];
    values.extend_from_slice(extra);
    Args::try_parse_from(values).unwrap()
}

#[test]
fn standalone_authority_advertises_reviewed_plan_lifecycle() {
    let capabilities = declared_capabilities_with_profiles(&standalone_capability_profile());
    for capability in [REVIEWED_PLAN_CAPABILITY, CONVERSATION_ACTIVITY_CAPABILITY] {
        assert!(capabilities.0.contains(&capability.into()));
    }
}

#[test]
fn omitted_local_runtime_does_not_block_claude_or_codex_standalone_bootstrap() {
    let args = args(&[]);
    validate(&args).unwrap();
    assert!(
        claurst_runtime_config(&args, std::path::Path::new("/tmp/gent"), None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn paired_local_runtime_paths_create_the_private_lazy_factory_config() {
    let args = args(&[
        "--standalone-claurst-executable",
        "/bin/sh",
        "--standalone-llama-server-executable",
        "/bin/sh",
    ]);
    validate(&args).unwrap();
    let config = claurst_runtime_config(&args, std::path::Path::new("/tmp/gent"), None)
        .unwrap()
        .unwrap();
    assert_eq!(
        config.request.claurst_home,
        std::path::Path::new("/tmp/gent/claurst")
    );
}

#[test]
fn provided_local_runtime_path_must_be_a_file() {
    let args = args(&[
        "--standalone-claurst-executable",
        "/missing/claurst",
        "--standalone-llama-server-executable",
        "/bin/sh",
    ]);
    assert!(validate(&args).unwrap_err().contains("Claurst"));
}

#[test]
fn local_runtime_paths_must_be_paired() {
    let args = args(&["--standalone-claurst-executable", "/bin/sh"]);
    assert!(
        validate(&args)
            .unwrap_err()
            .contains("must be supplied together")
    );
}

#[test]
fn provider_paths_must_be_paired() {
    let parsed = Args::try_parse_from([
        "gentd",
        "--standalone-authority",
        "--standalone-claude-executable",
        "/bin/sh",
    ])
    .unwrap();
    assert!(validate(&parsed).is_err());
}

#[test]
fn supplied_provider_paths_do_not_require_a_node_runtime() {
    let parsed = args(&[]);
    validate(&parsed).unwrap();
    assert_eq!(parsed.standalone_claude_executable, Some("/bin/sh".into()));
    assert_eq!(parsed.standalone_codex_executable, Some("/bin/sh".into()));
}

#[cfg(unix)]
#[tokio::test]
async fn fresh_standalone_bootstrap_binds_after_recovery_is_ready() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("gentd.sock");
    let data_dir = directory.path().to_string_lossy().into_owned();
    let socket_path = socket.to_string_lossy().into_owned();
    let args = args(&[
        "--data-dir",
        &data_dir,
        "--socket",
        &socket_path,
        "--standalone-claurst-executable",
        "/bin/sh",
        "--standalone-llama-server-executable",
        "/bin/sh",
    ]);

    tokio::task::LocalSet::new()
        .run_until(async {
            let mut task = tokio::task::spawn_local(super::run(args));
            tokio::select! {
                result = &mut task => panic!("standalone bootstrap stopped before binding: {result:?}"),
                result = tokio::time::timeout(Duration::from_secs(5), async {
                while !socket.exists() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                }) => result.unwrap(),
            }
            assert!(tokio::net::UnixStream::connect(&socket).await.is_ok());
            task.abort();
        })
        .await;
}
