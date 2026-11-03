use gent_drivers::{
    ProcessLauncher,
    codex_control::{CodexControlDecision, CodexControlRequest},
    codex_runner::CodexAppServerRunner,
};
use gent_ports::{AgentChatWorkspaceLedger, Ledger};
use gent_store::SqliteLedger;
use gent_types::{PermissionRequest, SandboxPermissionBinding};

pub(super) fn respond<L: ProcessLauncher>(
    runner: &mut CodexAppServerRunner<L, L::Process>,
    ledger: &SqliteLedger,
    binding: &SandboxPermissionBinding,
    request: CodexControlRequest,
) -> Result<(), String> {
    let ingress = ledger.host_ingress().map_err(|_| "host-ingress")?;
    if ingress.epoch != binding.host_epoch() || ingress.mode != gent_ports::IngressMode::Open {
        return Err("stale-host-ingress".into());
    }
    let workspace = ledger
        .agent_chat_workspace_for_run(binding.conversation_id(), binding.run_id())
        .map_err(|_| "run-workspace")?;
    let policy = crate::permission_workspace::policy_for(ledger, &workspace.workspace_id)
        .map_err(|_| "workspace-policy")?;
    if &policy != binding.policy() {
        return Err("stale-policy".into());
    }
    let category = crate::permission_category::for_tool(&request.tool_name);
    let normalized =
        PermissionRequest::new(request.tool_name.clone(), category, request.input, None);
    let decision = crate::permission_preflight::evaluate_callback(
        &policy,
        &normalized,
        binding.run_id(),
        binding.conversation_id(),
        ingress.epoch,
        runner.sandbox_evidence(binding.run_id()),
        false,
    );
    let native = match decision {
        crate::permission_preflight::PermissionPreflight::Allow => CodexControlDecision::Allow,
        crate::permission_preflight::PermissionPreflight::Deny => CodexControlDecision::Deny,
        crate::permission_preflight::PermissionPreflight::Ask => {
            println!("smoke-policy-control category={category:?} decision=ask");
            return Err("manual-decision-required-no-autoapproval".into());
        }
    };
    println!("smoke-policy-control category={category:?} decision={native:?}");
    runner
        .respond_control(binding.run_id(), &request.request_key, native, None)
        .map_err(|_| "control-response".into())
}
