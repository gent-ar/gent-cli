use crate::agent_chat_side_question_runners::AgentChatSideQuestionRunnerSources;
use crate::provider_executables::ProviderExecutables;
use gent_types::AgentChatProvider;

fn executable(directory: &std::path::Path, name: &str) -> std::path::PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

#[test]
fn resolves_claude_from_an_explicit_executable() {
    let directory = tempfile::tempdir().unwrap();
    let sources = AgentChatSideQuestionRunnerSources {
        data_dir: directory.path().to_path_buf(),
        executables: ProviderExecutables::explicit(
            Some(executable(directory.path(), "claude")),
            None,
        ),
        claurst_bridge: None,
    };
    assert!(sources.resolve(AgentChatProvider::Claude, None).is_ok());
}

#[test]
fn resolves_codex_from_an_explicit_executable_with_no_workspace_path() {
    let directory = tempfile::tempdir().unwrap();
    let sources = AgentChatSideQuestionRunnerSources {
        data_dir: directory.path().to_path_buf(),
        executables: ProviderExecutables::explicit(
            None,
            Some(executable(directory.path(), "codex")),
        ),
        claurst_bridge: None,
    };
    assert!(sources.resolve(AgentChatProvider::Codex, None).is_ok());
}

#[test]
fn fails_gracefully_when_claude_is_not_installed() {
    let directory = tempfile::tempdir().unwrap();
    let sources = AgentChatSideQuestionRunnerSources {
        data_dir: directory.path().to_path_buf(),
        executables: ProviderExecutables::explicit(None, None),
        claurst_bridge: None,
    };
    assert!(sources.resolve(AgentChatProvider::Claude, None).is_err());
}

#[test]
fn fails_gracefully_when_claurst_is_not_attached() {
    let directory = tempfile::tempdir().unwrap();
    let sources = AgentChatSideQuestionRunnerSources {
        data_dir: directory.path().to_path_buf(),
        executables: ProviderExecutables::explicit(None, None),
        claurst_bridge: None,
    };
    assert!(sources.resolve(AgentChatProvider::Claurst, None).is_err());
}
