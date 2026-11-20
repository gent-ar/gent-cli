use super::{command_grant_for, command_grant_permits, validate_command_grant};
use crate::{CommandGrant, CommandPatternError, PermissionCategory, PermissionRequest};
use serde_json::{Value, json};

fn request(tool: &str, command: Value) -> PermissionRequest {
    PermissionRequest::new(
        tool.into(),
        PermissionCategory::Command,
        Some(json!({ "command": command })),
        None,
    )
}

fn bash(command: &str) -> PermissionRequest {
    request("Bash", Value::String(command.into()))
}

fn grant(pattern: &str) -> CommandGrant {
    command_grant_for(&bash(pattern.trim_end_matches(" *")), pattern).unwrap()
}

#[test]
fn exact_and_prefix_patterns_match_only_their_argv() {
    let status = grant("git status");
    assert_eq!(status.words, ["git", "status"]);
    assert!(!status.any_arguments);
    assert!(command_grant_permits(&status, &bash("git status")));
    assert!(command_grant_permits(&status, &bash("  git\tstatus ")));
    assert!(!command_grant_permits(&status, &bash("git status --short")));
    assert!(!command_grant_permits(&status, &bash("git statusx")));
    assert!(!command_grant_permits(
        &status,
        &bash("/usr/bin/git status")
    ));
    let run = grant("npm run *");
    assert!(run.any_arguments);
    for allowed in [
        "npm run",
        "npm run build",
        "npm run test -- --watch",
        "npm run 'a b'",
    ] {
        assert!(command_grant_permits(&run, &bash(allowed)), "{allowed}");
    }
    for refused in ["npm", "npm install", "npx run build", "npm runx"] {
        assert!(!command_grant_permits(&run, &bash(refused)), "{refused}");
    }
}

#[test]
fn a_prefix_rule_never_approves_an_appended_or_hidden_command() {
    let rule = grant("git *");
    for command in [
        "git status && rm -rf ~",
        "git status || rm -rf ~",
        "git status; rm -rf ~",
        "git status & rm -rf ~",
        "git status | sh",
        "git status |& sh",
        "git status\nrm -rf ~",
        "git status\r\nrm -rf ~",
        "git status > ~/.bashrc",
        "git status >> ~/.bashrc",
        "git status < /etc/passwd",
        "git status 2>&1",
        "git log $(rm -rf ~)",
        "git log `rm -rf ~`",
        "git log ${HOME}",
        "git log $HOME",
        "git log \"$(rm -rf ~)\"",
        "git log \"`id`\"",
        "git log \"a\\\"; rm -rf ~; \\\"\"",
        "git log \\'; rm -rf ~; '",
        "git log a'; rm -rf ~; '",
        "git log 'a'b",
        "git log 'unterminated",
        "git log \"unterminated",
        "git (rm -rf ~)",
        "git { rm -rf ~; }",
        "git log <(rm -rf ~)",
        "git log # ; rm -rf ~",
        "git log \\",
        "git log @(rm)",
        "git log --% ; rm",
        "git log \u{2018}; rm -rf ~; \u{2019}",
        "git log \u{201c}x\u{201d}; rm",
        "git log \u{0}",
    ] {
        assert!(!command_grant_permits(&rule, &bash(command)), "{command:?}");
    }
}

#[test]
fn compound_requests_cannot_be_granted_by_any_pattern() {
    for command in ["git status && rm -rf ~", "ls | sh", "echo $(id)"] {
        assert_eq!(
            command_grant_for(&bash(command), "git *"),
            Err(CommandPatternError::NotMatchable),
            "{command}"
        );
    }
}

#[test]
fn shell_wrappers_are_matched_on_their_single_script() {
    let rule = grant("git *");
    assert!(command_grant_permits(&rule, &bash("bash -lc 'git status'")));
    assert!(!command_grant_permits(
        &rule,
        &request("Command", json!(["/bin/bash", "-lc", "git status"])),
    ));
    let codex = command_grant_for(
        &request("Command", json!(["/bin/bash", "-lc", "git status"])),
        "git *",
    )
    .unwrap();
    assert!(command_grant_permits(
        &codex,
        &request("Command", json!(["bash", "-c", "git diff"]))
    ));
    for script in ["git status; rm -rf ~", "git status && sh"] {
        assert!(!command_grant_permits(
            &codex,
            &request("Command", json!(["bash", "-lc", script]))
        ));
    }
    assert!(!command_grant_permits(
        &codex,
        &request(
            "Command",
            json!(["bash", "-lc", "bash -c 'git status; rm'"])
        )
    ));
    assert!(command_grant_permits(
        &codex,
        &request("Command", json!(["git", "status; rm -rf ~"]))
    ));
}

#[test]
fn patterns_that_would_match_everything_are_rejected() {
    for pattern in [
        "",
        "   ",
        "*",
        "bash *",
        "/bin/sh *",
        "SUDO.EXE *",
        "env *",
        "xargs *",
    ] {
        let target = bash("bash -c x");
        let error = command_grant_for(&target, pattern).unwrap_err();
        assert_eq!(error, CommandPatternError::TooBroad, "{pattern:?}");
    }
    assert!(
        validate_command_grant(&CommandGrant {
            tool_name: "Bash".into(),
            words: vec![],
            any_arguments: false,
        })
        .is_err()
    );
}

#[test]
fn invalid_patterns_return_a_typed_code() {
    for pattern in [
        "git * status",
        "git st*",
        "git status && *",
        "git $X",
        "FOO=1 git *",
        "git ~/x",
        "git '*'",
        "git 'a",
    ] {
        assert_eq!(
            command_grant_for(&bash("git status"), pattern),
            Err(CommandPatternError::Invalid),
            "{pattern:?}"
        );
    }
    assert_eq!(
        command_grant_for(&bash("git status"), "npm *"),
        Err(CommandPatternError::Mismatch)
    );
}

#[test]
fn rules_are_bound_to_their_tool_and_the_command_category() {
    let rule = grant("git *");
    assert!(!command_grant_permits(
        &rule,
        &request("PowerShell", Value::String("git status".into()))
    ));
    let mut read = bash("git status");
    read.category = PermissionCategory::Read;
    assert!(!command_grant_permits(&rule, &read));
    assert_eq!(
        command_grant_for(&read, "git *"),
        Err(CommandPatternError::NotCommand)
    );
    let no_command = PermissionRequest::new(
        "Permission".into(),
        PermissionCategory::Command,
        Some(json!({"permissions": {}})),
        None,
    );
    assert_eq!(
        command_grant_for(&no_command, "git *"),
        Err(CommandPatternError::NotCommand)
    );
}

#[test]
fn powershell_paths_and_quoted_arguments_stay_literal() {
    let request = |text: &str| request("PowerShell", Value::String(text.into()));
    let rule = command_grant_for(&request("Get-Content C:\\logs\\a.txt"), "Get-Content *").unwrap();
    assert!(command_grant_permits(
        &rule,
        &request("Get-Content 'C:\\x y.txt'")
    ));
    for command in [
        "Get-Content x; Remove-Item y",
        "Get-Content x `; Remove-Item y",
        "Get-Content $env:USERPROFILE",
        "Get-Content (Remove-Item y)",
        "Get-Content x | Invoke-Expression",
        "Get-Content @args",
        "Get-Content \"$(rm y)\"",
    ] {
        assert!(
            !command_grant_permits(&rule, &request(command)),
            "{command}"
        );
    }
}
