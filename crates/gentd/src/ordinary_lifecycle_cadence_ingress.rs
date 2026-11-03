use super::{
    AgentChatProvider, InterruptGoalStop, OrdinaryPromptAdmissionError, OrdinaryPromptIngress,
    OrdinaryPromptPermit, PendingClaurstInterrupt,
};
use std::sync::atomic::Ordering;

impl<L> OrdinaryPromptIngress<L> {
    pub(crate) fn acquire_prompt(
        &self,
    ) -> Result<OrdinaryPromptPermit, OrdinaryPromptAdmissionError> {
        self.control.acquire_prompt()
    }

    pub(crate) fn respond_codex_permission(
        &self,
        run_id: &str,
        request_id: &str,
        decision: gent_drivers::codex_control::CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), String> {
        self.wake
            .router
            .lock()
            .map_err(|_| "standalone lifecycle router is unavailable".to_owned())?
            .respond_codex_permission(run_id, request_id, decision, answers)
            .map_err(|_| "Codex permission owner is unavailable".to_owned())
    }

    pub(crate) fn respond_claude_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), String> {
        self.respond_claude_permission_with_input(
            run_id,
            request_id,
            behavior,
            persist_suggestions,
            None,
        )
    }

    pub(crate) fn respond_claude_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), String> {
        self.wake
            .router
            .lock()
            .map_err(|_| "standalone lifecycle router is unavailable".to_owned())?
            .respond_claude_permission_with_input(
                run_id,
                request_id,
                behavior,
                persist_suggestions,
                updated_input,
            )
            .map_err(|_| "Claude permission owner is unavailable".to_owned())
    }

    pub(crate) fn interrupt_run(
        &self,
        provider: AgentChatProvider,
        run_id: &str,
    ) -> Result<(), String> {
        if let Some(readiness) = &self.standalone_readiness
            && readiness.cancel_held_prompts(run_id)?
        {
            return Ok(());
        }
        if provider == AgentChatProvider::Claurst {
            if !self.async_claurst_attached.load(Ordering::Acquire) {
                return Err("Claurst interrupt owner is unavailable".into());
            }
            self.async_claurst_interrupts
                .lock()
                .map_err(|_| "Claurst interrupt queue is unavailable".to_owned())?
                .push_back(PendingClaurstInterrupt {
                    run_id: run_id.to_owned(),
                    expected_turn_id: None,
                    goal_stop: None,
                });
            if let Some(notify) = &self.async_claurst_notify {
                notify.notify_one();
            }
            return Ok(());
        }
        self.wake
            .router
            .lock()
            .map_err(|_| "standalone lifecycle router is unavailable".to_owned())?
            .interrupt_run(provider, run_id)
            .map_err(|_| "provider interrupt owner is unavailable".to_owned())
    }

    pub(crate) fn interrupt_turn(
        &self,
        provider: AgentChatProvider,
        run_id: &str,
        turn_id: &str,
        goal_stop: InterruptGoalStop,
    ) -> Result<(), String> {
        if turn_id.trim().is_empty() {
            return Err("expected interrupt turn is empty".into());
        }
        if let Some(readiness) = &self.standalone_readiness
            && readiness.cancel_held_turn(run_id, turn_id)?
        {
            goal_stop.apply();
            return Ok(());
        }
        if provider == AgentChatProvider::Claurst {
            if !self.async_claurst_attached.load(Ordering::Acquire) {
                return Err("Claurst interrupt owner is unavailable".into());
            }
            self.async_claurst_interrupts
                .lock()
                .map_err(|_| "Claurst interrupt queue is unavailable")?
                .push_back(PendingClaurstInterrupt {
                    run_id: run_id.to_owned(),
                    expected_turn_id: Some(turn_id.to_owned()),
                    goal_stop: Some(goal_stop),
                });
            if let Some(notify) = &self.async_claurst_notify {
                notify.notify_one();
            }
            return Ok(());
        }
        self.wake
            .router
            .lock()
            .map_err(|_| "standalone lifecycle router is unavailable")?
            .interrupt_turn(provider, run_id, turn_id)
            .map_err(|_| "the interrupt target turn is no longer active")?;
        goal_stop.apply();
        Ok(())
    }
}
