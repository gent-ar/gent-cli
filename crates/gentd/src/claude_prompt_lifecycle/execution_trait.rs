use super::{ClaudePromptRunner, ClaudePromptStart};
use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::interrupt::ProcessTreeSignal;
use gent_drivers::supervisor::{ProcessLauncher, ProviderProcess};
use gent_ports::{PublicProviderRunError, PublicProviderRunner};

pub(crate) trait ClaudePromptExecution: PublicProviderRunner {
    fn validate_permission_binding(
        &self,
        binding: gent_types::SandboxPermissionBinding,
    ) -> Result<(), PublicProviderRunError> {
        crate::permission_workspace::require_supported_launch(
            binding.policy(),
            gent_types::AgentChatProvider::Claude,
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
    fn prepare_claude_prompt(
        &self,
        run_id: String,
        prompt: ClaudePromptStart,
    ) -> Result<(), PublicProviderRunError>;
    fn cancel_claude_prompt(&self, run_id: &str);
    fn poll_claude_prompt(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<ClaudeRunnerEffect>>, PublicProviderRunError>;
    fn has_claude_session(&self, run_id: &str) -> bool;
    fn release_claude_session(&self, run_id: &str) -> Result<(), PublicProviderRunError>;
    fn submit_claude_prompt(
        &self,
        run_id: &str,
        prompt: &str,
        goal: Option<&gent_types::GoalProjection>,
        content: &[serde_json::Value],
    ) -> Result<(), PublicProviderRunError>;
    fn steer_claude_prompt(
        &self,
        run_id: &str,
        message_id: &str,
        prompt: &str,
        content: &[serde_json::Value],
    ) -> Result<(), PublicProviderRunError>;
    fn signal_claude_process(
        &self,
        run_id: &str,
        signal: ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError>;
    fn respond_claude_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), PublicProviderRunError>;

    fn respond_claude_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), PublicProviderRunError> {
        let _ = updated_input;
        self.respond_claude_permission(run_id, request_id, behavior, persist_suggestions)
    }
}

impl<L, P> ClaudePromptExecution for ClaudePromptRunner<L, P>
where
    L: ProcessLauncher<Process = P> + Send + Sync,
    P: ProviderProcess + Send,
{
    fn validate_permission_binding(
        &self,
        binding: gent_types::SandboxPermissionBinding,
    ) -> Result<(), PublicProviderRunError> {
        super::lock(&self.runner)
            .prepare_permission_binding(binding)
            .map_err(super::map_error)
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
        let runner = super::lock(&self.runner);
        let selected = super::lock(&self.launched_mcp);
        let trusted = runner.is_live(run_id)
            && crate::internal_mcp_trust::permits(
                selected
                    .get(run_id)
                    .filter(|(conversation, _)| conversation == conversation_id)
                    .map(|(_, servers)| servers),
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
            runner.sandbox_evidence(run_id),
            trusted,
        )
    }
    fn prepare_claude_prompt(
        &self,
        run_id: String,
        prompt: ClaudePromptStart,
    ) -> Result<(), PublicProviderRunError> {
        self.prepare(run_id, prompt)
    }
    fn cancel_claude_prompt(&self, run_id: &str) {
        self.cancel(run_id);
        self.cleanup_config(run_id);
    }
    fn poll_claude_prompt(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<ClaudeRunnerEffect>>, PublicProviderRunError> {
        self.poll(run_id)
    }
    fn has_claude_session(&self, run_id: &str) -> bool {
        self.owns(run_id)
    }
    fn release_claude_session(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        self.release(run_id)
    }
    fn submit_claude_prompt(
        &self,
        run_id: &str,
        prompt: &str,
        goal: Option<&gent_types::GoalProjection>,
        content: &[serde_json::Value],
    ) -> Result<(), PublicProviderRunError> {
        self.submit(run_id, prompt, goal, content)
    }
    fn steer_claude_prompt(
        &self,
        run_id: &str,
        message_id: &str,
        prompt: &str,
        content: &[serde_json::Value],
    ) -> Result<(), PublicProviderRunError> {
        super::lock(&self.runner)
            .steer(run_id, message_id, prompt, content)
            .map_err(super::map_error)
    }
    fn signal_claude_process(
        &self,
        run_id: &str,
        signal: ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError> {
        let result = super::lock(&self.runner)
            .signal(run_id, signal)
            .map_err(super::map_error);
        if result.is_ok() {
            self.cleanup_config(run_id);
        }
        result
    }
    fn respond_claude_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), PublicProviderRunError> {
        self.respond_permission(run_id, request_id, behavior, persist_suggestions)
    }
    fn respond_claude_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), PublicProviderRunError> {
        self.respond_permission_with_input(
            run_id,
            request_id,
            behavior,
            persist_suggestions,
            updated_input,
        )
    }
}
