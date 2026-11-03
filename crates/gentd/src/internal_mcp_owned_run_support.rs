//! Concrete production execution/preflight paths with inert process I/O, never containment proof.
use gent_drivers::{
    buffering::BufferPolicy,
    interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal},
    supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError},
};
use gent_types::*;
use std::{
    collections::VecDeque,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
pub(super) struct State {
    pub(super) alive: AtomicBool,
    pub(super) fail: AtomicBool,
    reads: Mutex<VecDeque<Vec<u8>>>,
}
#[derive(Clone)]
pub(super) struct Process(Arc<State>);
impl ProcessTreeControl for Process {
    fn signal_tree(&self, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        self.0.alive.store(false, Ordering::SeqCst);
        Ok(())
    }
}
impl ProviderProcess for Process {
    fn write_frame(&self, _: &[u8]) -> Result<(), ProcessTreeError> {
        Ok(())
    }
    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        Ok(())
    }
    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        Ok(self.0.reads.lock().unwrap().pop_front())
    }
    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        Ok((!self.0.alive.load(Ordering::SeqCst)).then_some(Some(0)))
    }
}
#[derive(Clone)]
pub(super) struct Launcher(pub(super) Arc<State>);
impl ProcessLauncher for Launcher {
    type Process = Process;
    fn launch(&self, launch: &ProviderLaunch) -> Result<Process, SupervisorError> {
        if self.0.fail.load(Ordering::SeqCst) {
            return Err(SupervisorError::Launch("fixture failure".into()));
        }
        // Upstream can already have sent a callback before any start/initialize acknowledgment.
        // The lifecycle cannot consume it until the owning runner's start call returns.
        if launch.provider == "claude" {
            self.0.reads.lock().unwrap().push_back(br#"{"type":"control_request","request_id":"early","request":{"subtype":"can_use_tool","tool_use_id":"tool","tool_name":"mcp__gent_goal__gent_goal_update","input":{}}}
"#.to_vec());
        }
        self.0.alive.store(true, Ordering::SeqCst);
        Ok(Process(self.0.clone()))
    }
}
pub(super) fn policy() -> PolicyRecord {
    PolicyRecord {
        policy_id: "policy".into(),
        workspace_id: "workspace".into(),
        scope: PolicyScope::ProviderPermissions,
        revision: 1,
        mode: PermissionMode::AskEveryTime,
        containment: gent_types::Containment::Sandboxed,
        allowed_tools: vec![],
        allowed_categories: vec![],
    }
}
pub(super) fn request(name: &str) -> PermissionRequest {
    PermissionRequest::new(name.into(), PermissionCategory::Provider, None, None)
}
pub(super) fn selection(provider: AgentChatProvider) -> AgentChatSelection {
    AgentChatSelection {
        provider,
        model: "fixture".into(),
        effort: AgentChatEffort::Low,
        mode: AgentChatMode::Agent,
    }
}
pub(super) fn buffer() -> BufferPolicy {
    BufferPolicy::new(4, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap()
}
pub(super) fn lock(root: &Path, provider: &str) -> RunVersionLock {
    let exe = root.join(provider);
    std::fs::write(&exe, "inert fixture image").unwrap();
    gent_drivers::lock::capture(provider, &exe, "fixture", "fixture").unwrap()
}
pub(super) fn config(root: &Path) -> (std::path::PathBuf, serde_json::Value) {
    let servers = injection(root).servers().clone();
    let path = root.join("standalone-mcp.json");
    std::fs::write(&path, serde_json::json!({"mcpServers":servers}).to_string()).unwrap();
    (path, servers)
}

pub(super) fn injection(root: &Path) -> crate::standalone_mcp_config::InjectedInternalMcp {
    crate::standalone_mcp_config::InjectedInternalMcp::fixture(root, "fixture-exact-daemon-gent")
}
