use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use gent_protocol::{
    LocalModelInstallState, ProviderAuthFrame,
    model_catalog::{LocalModelAvailability, ProviderAvailability},
};
use gent_types::{AgentChatEffort, ProviderAuthLifecycle, ProviderAuthStatus};

use super::{
    claude::ClaudeModelSource, claude_initialize::ClaudeInitialize, codex::CodexModelSource,
    local::LocalModelSource, service::ModelCatalogSource,
};
use crate::{
    provider_auth_api::ProviderAuthPort,
    provider_launch_budget::{ProviderLaunchError, warm_first_execution},
};

const FAKE_CODEX: &str = include_str!("../testdata/fake-codex-model-list.py");
const FAKE_CLAUDE: &str = include_str!("../testdata/fake-claude-initialize.py");

struct FixedAuth(ProviderAuthLifecycle);

impl ProviderAuthPort for FixedAuth {
    fn exchange(&self, frame: ProviderAuthFrame) -> Result<ProviderAuthFrame, String> {
        let ProviderAuthFrame::StatusRequest {
            request_id,
            provider,
        } = frame
        else {
            return Err("unexpected auth frame".into());
        };
        Ok(ProviderAuthFrame::Status {
            request_id,
            status: self.check_status(provider),
        })
    }

    fn check_status(&self, provider: gent_types::ProviderAuthProvider) -> ProviderAuthStatus {
        ProviderAuthStatus {
            challenge_id: None,
            provider,
            binary_lock: None,
            lifecycle: self.0,
            selected_method: None,
            expires_at_unix_seconds: None,
        }
    }
}

fn executable(root: &Path, name: &str, source: &str) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, source).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    warm_first_execution(&path);
    path
}

fn claude_initialize(root: &Path) -> Arc<ClaudeInitialize> {
    ClaudeInitialize::new(
        crate::provider_executables::ProviderExecutables::explicit(
            Some(executable(root, "claude", FAKE_CLAUDE)),
            None,
        ),
        Duration::from_secs(10),
    )
}

fn codex(root: &Path, lifecycle: ProviderAuthLifecycle) -> CodexModelSource {
    CodexModelSource {
        lifecycle_control: None,
        executables: crate::provider_executables::ProviderExecutables::explicit(
            None,
            Some(executable(root, "codex", FAKE_CODEX)),
        ),
        auth: Arc::new(FixedAuth(lifecycle)),
        timeout: Duration::from_secs(10),
    }
}

#[test]
fn codex_models_come_from_every_model_list_page_with_supported_efforts_only() {
    let root = tempfile::tempdir().unwrap();
    let listing = codex(root.path(), ProviderAuthLifecycle::Authenticated)
        .load()
        .unwrap();

    assert_eq!(listing.availability, ProviderAvailability::Ready);
    let ids = listing
        .models
        .iter()
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["gpt-6-astra", "gpt-5.6-luna", "default"]);
    assert!(ids.contains(&"default"));
    assert!(!ids.contains(&"gpt-internal"));
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        ids.len()
    );
    let astra = &listing.models[0];
    assert!(astra.is_default);
    assert_eq!(astra.label, "GPT-6-Astra");
    assert_eq!(
        astra.efforts,
        [
            AgentChatEffort::Low,
            AgentChatEffort::Medium,
            AgentChatEffort::Ultra
        ]
    );
    assert_eq!(astra.default_effort, Some(AgentChatEffort::Medium));
    assert!(!listing.models[1].is_default);
    assert_eq!(listing.models[1].default_effort, None);
    let requests = fs::read_to_string(root.path().join("model-list-requests.jsonl")).unwrap();
    let lists = requests
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|request| request["method"] == "model/list")
        .collect::<Vec<_>>();
    assert_eq!(lists.len(), 2);
    assert_eq!(lists[0]["params"]["includeHidden"], false);
    assert_eq!(lists[1]["params"]["cursor"], "page-2");
}

#[test]
fn codex_signed_out_is_never_probed_and_list_errors_stay_typed() {
    let root = tempfile::tempdir().unwrap();
    let signed_out = codex(root.path(), ProviderAuthLifecycle::Unauthenticated)
        .load()
        .unwrap();
    assert_eq!(signed_out.availability, ProviderAvailability::SignedOut);
    assert!(!root.path().join("model-list-requests.jsonl").exists());
    fs::write(root.path().join("model-list-error"), "").unwrap();
    let error = codex(root.path(), ProviderAuthLifecycle::Authenticated)
        .load()
        .err()
        .unwrap();
    assert!(
        matches!(&error, ProviderLaunchError::Failed(message) if message.contains("not signed in")),
        "{error}"
    );
}

#[test]
fn a_missing_provider_lists_no_models_of_its_own_before_install() {
    let listing = CodexModelSource {
        lifecycle_control: None,
        executables: crate::provider_executables::ProviderExecutables::explicit(None, None),
        auth: Arc::new(FixedAuth(ProviderAuthLifecycle::NotInstalled)),
        timeout: Duration::from_secs(1),
    }
    .load()
    .unwrap();
    assert_eq!(listing.availability, ProviderAvailability::NotInstalled);
    assert!(listing.models.is_empty());
    let claude = ClaudeModelSource {
        initialize: ClaudeInitialize::new(
            crate::provider_executables::ProviderExecutables::explicit(None, None),
            Duration::from_secs(1),
        ),
        auth: Arc::new(FixedAuth(ProviderAuthLifecycle::NotInstalled)),
    }
    .load()
    .unwrap();
    assert_eq!(claude.availability, ProviderAvailability::NotInstalled);
    assert!(claude.models.is_empty());
}

#[test]
fn claude_models_come_only_from_the_installed_provider() {
    let root = tempfile::tempdir().unwrap();
    let listing = ClaudeModelSource {
        initialize: claude_initialize(root.path()),
        auth: Arc::new(FixedAuth(ProviderAuthLifecycle::Authenticated)),
    }
    .load()
    .unwrap();

    let ids = listing
        .models
        .iter()
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["default", "sonnet", "haiku"]);
    assert!(listing.models[0].is_default);
    assert_eq!(listing.models[0].label, "Default (recommended)");
    assert_eq!(listing.models[0].efforts.len(), 5);
    assert_eq!(listing.models[1].default_effort, None);
    assert!(listing.models[2].efforts.is_empty());
    assert_eq!(listing.models[2].default_effort, None);
}

#[test]
fn claude_commands_come_from_the_same_initialize_probe_run_in_the_workspace() {
    use gent_protocol::agent_chat_commands::CommandListing;
    use gent_types::{AgentChatCommandDispatch, AgentChatCommandOrigin, AgentChatProvider};
    let root = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let initialize = claude_initialize(root.path());
    let read = || {
        super::commands::provider_commands(
            &initialize,
            AgentChatProvider::Claude,
            Some(workspace.path().to_path_buf()),
            false,
        )
    };
    assert_eq!(read().listing, CommandListing::Loading);
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let listed = loop {
        let listed = read();
        if listed.listing != CommandListing::Loading || std::time::Instant::now() > deadline {
            break listed;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(listed.listing, CommandListing::Ready);
    let names: Vec<&str> = listed
        .commands
        .iter()
        .map(|command| command.name.as_str())
        .collect();
    assert_eq!(names, ["context", "config", "workspace-skill"]);
    let provider = AgentChatProvider::Claude;
    assert_eq!(
        listed.commands[0].dispatch,
        AgentChatCommandDispatch::ProviderNative
    );
    assert!(matches!(
        listed.commands[1].dispatch,
        AgentChatCommandDispatch::Unsupported { .. }
    ));
    assert_eq!(
        listed.commands[2].origin,
        AgentChatCommandOrigin::ProviderSkill { provider }
    );
    let canonical = fs::canonicalize(workspace.path()).unwrap();
    assert!(
        listed.commands[2]
            .description
            .ends_with(&*canonical.to_string_lossy())
    );
    assert_eq!(
        super::commands::provider_commands(&initialize, AgentChatProvider::Codex, None, false),
        super::commands::ProviderCommands::none()
    );
}

#[path = "model_catalog_provider_additional_tests.rs"]
mod additional;
