//! Process adapter implemented only by the daemon-owned Codex prompt runner.

use gent_drivers::codex_control::CodexControlDecision;
use gent_drivers::codex_prompt_runner::{CodexPromptRunner, CodexPromptStart};
use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::interrupt::ProcessTreeSignal;
use gent_drivers::supervisor::{ProcessLauncher, ProviderProcess};
use gent_ports::{PublicProviderRunError, PublicProviderRunner};
use gent_types::GoalProjection;

pub(crate) trait CodexPromptExecution: PublicProviderRunner {
    fn validate_permission_binding(
        &self,
        binding: gent_types::SandboxPermissionBinding,
    ) -> Result<(), PublicProviderRunError> {
        crate::permission_workspace::require_supported_launch(
            binding.policy(),
            gent_types::AgentChatProvider::Codex,
        )
    }

    fn permission_preflight(
        &self,
        _context: &crate::permission_preflight::PreflightContext<'_>,
        policy: &gent_types::PolicyRecord,
        request: &gent_types::PermissionRequest,
    ) -> crate::permission_preflight::PermissionPreflight {
        crate::permission_preflight::evaluate(policy, request)
    }
    fn prepare_codex_prompt(
        &self,
        run_id: String,
        conversation_id: String,
        prompt: CodexPromptStart,
    ) -> Result<(), PublicProviderRunError>;
    fn cancel_codex_prompt(&self, run_id: &str);
    fn poll_codex_prompt(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<CodexRunnerEffect>>, PublicProviderRunError>;
    fn has_codex_session(&self, run_id: &str) -> bool;
    fn release_codex_session(&self, run_id: &str) -> Result<(), PublicProviderRunError>;
    fn refresh_codex_mcp_config(&self, run_id: &str) -> Result<bool, PublicProviderRunError>;
    fn codex_mcp_servers(&self) -> Result<Option<serde_json::Value>, PublicProviderRunError>;
    fn submit_codex_prompt(
        &self,
        run_id: &str,
        prompt: &str,
        goal: Option<&GoalProjection>,
        attachments: &[serde_json::Value],
        interrupted_reply: Option<&str>,
    ) -> Result<(), PublicProviderRunError>;
    fn signal_codex_process(
        &self,
        run_id: &str,
        signal: ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError>;
    fn interrupt_codex_turn(&self, run_id: &str) -> Result<(), PublicProviderRunError>;
    fn steer_codex_turn(
        &self,
        run_id: &str,
        message_id: &str,
        prompt: &str,
        attachments: &[serde_json::Value],
    ) -> Result<(), PublicProviderRunError>;
    fn respond_codex_control(
        &self,
        run_id: &str,
        request_id: &str,
        decision: CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), PublicProviderRunError>;
}

impl<L, P> CodexPromptExecution for CodexPromptRunner<L, P>
where
    L: ProcessLauncher<Process = P> + Send + Sync,
    P: ProviderProcess + Send,
{
    fn validate_permission_binding(
        &self,
        binding: gent_types::SandboxPermissionBinding,
    ) -> Result<(), PublicProviderRunError> {
        self.prepare_permission_binding(binding)
    }

    fn permission_preflight(
        &self,
        context: &crate::permission_preflight::PreflightContext<'_>,
        policy: &gent_types::PolicyRecord,
        request: &gent_types::PermissionRequest,
    ) -> crate::permission_preflight::PermissionPreflight {
        let crate::permission_preflight::PreflightContext {
            run_id,
            conversation_id,
            disallowed,
            injection,
            host_epoch,
        } = *context;
        self.with_owned_mcp(run_id, conversation_id, |actual, _root, proof| {
            let trusted = crate::internal_mcp_trust::permits(
                actual,
                injection,
                conversation_id,
                &request.tool_name,
                disallowed,
            );
            crate::permission_preflight::evaluate_callback(
                policy,
                request,
                run_id,
                conversation_id,
                host_epoch,
                proof,
                trusted,
            )
        })
    }
    fn prepare_codex_prompt(
        &self,
        run_id: String,
        conversation_id: String,
        prompt: CodexPromptStart,
    ) -> Result<(), PublicProviderRunError> {
        self.prepare_for_conversation(run_id, conversation_id, prompt)
    }

    fn cancel_codex_prompt(&self, run_id: &str) {
        self.cancel(run_id);
    }

    fn poll_codex_prompt(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<CodexRunnerEffect>>, PublicProviderRunError> {
        self.poll(run_id)
    }

    fn has_codex_session(&self, run_id: &str) -> bool {
        self.owns(run_id)
    }

    fn release_codex_session(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        self.release_session(run_id)
    }

    fn refresh_codex_mcp_config(&self, run_id: &str) -> Result<bool, PublicProviderRunError> {
        self.refresh_mcp_config(run_id)
    }

    fn codex_mcp_servers(&self) -> Result<Option<serde_json::Value>, PublicProviderRunError> {
        Ok(self.current_mcp_servers()?.map(|(servers, _)| servers))
    }

    fn submit_codex_prompt(
        &self,
        run_id: &str,
        prompt: &str,
        goal: Option<&GoalProjection>,
        attachments: &[serde_json::Value],
        interrupted_reply: Option<&str>,
    ) -> Result<(), PublicProviderRunError> {
        self.submit(run_id, prompt, goal, attachments, interrupted_reply)
    }

    fn signal_codex_process(
        &self,
        run_id: &str,
        signal: ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError> {
        self.signal_process(run_id, signal)
    }

    fn interrupt_codex_turn(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        self.interrupt_turn(run_id)
    }

    fn steer_codex_turn(
        &self,
        run_id: &str,
        message_id: &str,
        prompt: &str,
        attachments: &[serde_json::Value],
    ) -> Result<(), PublicProviderRunError> {
        self.steer_turn(run_id, message_id, prompt, attachments)
    }

    fn respond_codex_control(
        &self,
        run_id: &str,
        request_id: &str,
        decision: CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), PublicProviderRunError> {
        self.respond_control(run_id, request_id, decision, answers)
    }
}
