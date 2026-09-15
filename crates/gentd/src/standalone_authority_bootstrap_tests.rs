use clap::Parser;
use gent_protocol::{
    AGENT_CHAT_PROJECTION_CAPABILITY, CONVERSATION_ACTIVITY_CAPABILITY,
    PROMPT_PROVIDER_PROVISION_CAPABILITY, PROVIDER_AUTH_CAPABILITY, PROVIDER_READINESS_CAPABILITY,
    REVIEWED_PLAN_CAPABILITY,
};
use gent_runtime::catalog::declared_capabilities_with_profiles;
#[cfg(unix)]
use std::time::Duration;

use super::{
    Args, claurst_runtime_config, standalone_capability_profile, validate, validate_build,
};
use crate::standalone_authority_release::{authority_source, packaged::PackagedAuthority};

const EXISTING_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");

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
fn prompt_provider_provision_is_advertised_only_with_a_signed_release() {
    let without = declared_capabilities_with_profiles(&standalone_capability_profile(false));
    let with = declared_capabilities_with_profiles(&standalone_capability_profile(true));
    assert!(
        !without
            .0
            .contains(&PROMPT_PROVIDER_PROVISION_CAPABILITY.into())
    );
    assert!(
        with.0
            .contains(&PROMPT_PROVIDER_PROVISION_CAPABILITY.into())
    );
}

#[test]
fn standalone_authority_loads_the_release_packaged_beside_gentd() {
    let packaged = PackagedAuthority {
        release: "/release/authority/ordinary-authority.json".into(),
        root_keys: vec![format!("root:{}", "01".repeat(32))],
    };
    let expected = packaged.clone();
    assert_eq!(
        authority_source(None, &[], move || Ok(Some(packaged))).unwrap(),
        Some(expected)
    );
    let explicit = args(&[
        "--standalone-authority-release",
        "/explicit/release.json",
        "--standalone-authority-key",
        "root:0101010101010101010101010101010101010101010101010101010101010101",
    ]);
    let selected = authority_source(
        explicit.standalone_authority_release.clone(),
        &explicit.standalone_authority_keys,
        || panic!("explicit authority wins"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        selected.release,
        std::path::Path::new("/explicit/release.json")
    );
}

#[test]
fn standalone_release_path_and_root_keys_must_be_paired() {
    let release_only = args(&["--standalone-authority-release", "/tmp/release.json"]);
    assert!(validate(&release_only).is_err());
    let key_only = args(&[
        "--standalone-authority-key",
        "root:0101010101010101010101010101010101010101010101010101010101010101",
    ]);
    assert!(validate(&key_only).is_err());
}

#[test]
fn verify_only_requires_standalone_mode_and_a_signed_release() {
    assert!(Args::try_parse_from(["gentd", "--verify-standalone-authority-release"]).is_err());
    let missing_release = args(&["--verify-standalone-authority-release"]);
    assert!(validate(&missing_release).is_err());
    let configured = args(&[
        "--verify-standalone-authority-release",
        "--standalone-authority-release",
        "/tmp/release.json",
        "--standalone-authority-key",
        "root:0101010101010101010101010101010101010101010101010101010101010101",
    ]);
    validate(&configured).unwrap();
}

#[test]
fn standalone_authority_advertises_reviewed_plan_lifecycle() {
    let capabilities = declared_capabilities_with_profiles(&standalone_capability_profile(false));
    for capability in [
        AGENT_CHAT_PROJECTION_CAPABILITY,
        REVIEWED_PLAN_CAPABILITY,
        CONVERSATION_ACTIVITY_CAPABILITY,
        PROVIDER_AUTH_CAPABILITY,
        PROVIDER_READINESS_CAPABILITY,
    ] {
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
        EXISTING_FILE,
        "--standalone-llama-server-executable",
        EXISTING_FILE,
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
        EXISTING_FILE,
    ]);
    assert!(validate(&args).unwrap_err().contains("Claurst"));
}

#[test]
fn local_runtime_paths_must_be_paired() {
    let args = args(&["--standalone-claurst-executable", EXISTING_FILE]);
    assert!(
        validate(&args)
            .unwrap_err()
            .contains("must be supplied together")
    );
}

#[test]
fn provider_paths_may_be_supplied_independently() {
    for arguments in [
        [
            "gentd",
            "--standalone-authority",
            "--standalone-claude-executable",
            "/bin/sh",
        ],
        [
            "gentd",
            "--standalone-authority",
            "--standalone-codex-executable",
            "/bin/sh",
        ],
    ] {
        let parsed = Args::try_parse_from(arguments).unwrap();
        validate(&parsed).unwrap();
    }
}

#[test]
fn explicit_provider_paths_bypass_installed_verification_only_in_development_builds() {
    let explicit = args(&[]);
    validate_build(&explicit, true).unwrap();
    assert!(
        validate_build(&explicit, false)
            .unwrap_err()
            .contains("development build")
    );
    let installed = Args::try_parse_from(["gentd", "--standalone-authority"]).unwrap();
    validate_build(&installed, false).unwrap();
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
