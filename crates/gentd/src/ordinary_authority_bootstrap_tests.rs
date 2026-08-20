use clap::Parser;

use super::{Args, validate_profile};

fn parsed(arguments: &[&str]) -> Args {
    Args::try_parse_from(arguments).unwrap()
}

fn ordinary() -> Vec<&'static str> {
    vec![
        "gentd",
        "--ordinary-authority",
        "--ordinary-authority-release",
        "release.json",
        "--ordinary-authority-key",
        "root:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ]
}

#[test]
fn ordinary_profile_requires_its_release_and_key_before_opening_state() {
    assert!(
        validate_profile(&parsed(&["gentd", "--ordinary-authority"]))
            .unwrap_err()
            .contains("release")
    );
    let mut arguments = ordinary();
    arguments.truncate(4);
    assert!(
        validate_profile(&parsed(&arguments))
            .unwrap_err()
            .contains("key")
    );
}

#[test]
fn ordinary_profile_rejects_other_authority_and_update_settings() {
    let mut arguments = ordinary();
    arguments.extend(["--runtime-update-check-authority"]);
    assert!(
        validate_profile(&parsed(&arguments))
            .unwrap_err()
            .contains("cannot combine")
    );
    let mut arguments = ordinary();
    arguments.extend(["--agent-chat-authority"]);
    assert!(
        validate_profile(&parsed(&arguments))
            .unwrap_err()
            .contains("mutually exclusive")
    );
}

#[test]
fn ordinary_profile_accepts_only_its_own_bootstrap_inputs() {
    assert!(validate_profile(&parsed(&ordinary())).is_ok());
}
