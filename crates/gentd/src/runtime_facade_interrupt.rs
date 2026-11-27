use gent_ports::{AgentChatReadLedger, ConversationLedger, RunLifecycleFactLedger};
use gent_protocol::{AgentChatIntentFrame, agent_chat_interrupt::InterruptTurnEnded};
use gent_types::{
    AgentChatConversationId, AgentChatRejection, AgentChatRunId, HostEpoch, ToolPhase,
};

use super::RuntimeFacade;
use crate::agent_chat_intent_error::AgentChatIntentError;

impl RuntimeFacade {
    pub(crate) fn running_turn_run(
        &self,
        conversation_id: &str,
    ) -> Result<Option<AgentChatRunId>, String> {
        let ledger = &self.transcript_import_ledger;
        let detail = ledger
            .read_agent_chat_detail(conversation_id)
            .map_err(|error| error.to_string())?;
        for run in detail.runs.into_iter().rev() {
            let turns = ledger
                .list_run_turns(&run.run_id)
                .map_err(|error| error.to_string())?;
            if turns.iter().any(|turn| !turn.phase.is_terminal()) {
                return Ok(Some(AgentChatRunId(run.run_id)));
            }
        }
        Ok(None)
    }

    pub(super) fn interrupt_intent(
        &self,
        host_epoch: HostEpoch,
        frame: AgentChatIntentFrame,
    ) -> Result<Option<AgentChatIntentFrame>, AgentChatIntentError> {
        let AgentChatIntentFrame::Interrupt {
            request_id,
            receipt_id,
            conversation_id,
            run_id,
            expected_turn_id,
        } = frame
        else {
            return Ok(None);
        };
        let running = self.running_turn_run(&conversation_id.0)?;
        let target = running.clone().unwrap_or_else(|| run_id.clone());
        if let Some(turn_id) = expected_turn_id.as_deref()
            && let Some(ended) = self.ended_turn(&conversation_id, &[&run_id, &target], turn_id)?
        {
            return Err(ended.into());
        }
        let ingress = self
            .ordinary_prompt_ingress
            .as_ref()
            .ok_or_else(|| "agent-chat provider lifecycle is not configured".to_owned())?;
        let selection = self
            .agent_chat_reads
            .as_ref()
            .ok_or_else(|| "agent-chat reads are unavailable".to_owned())?
            .run_selection(&conversation_id.0, &target.0)
            .map_err(|error| error.to_string())?;
        let mut interrupt_payload =
            serde_json::json!({ "conversationId": conversation_id, "runId": run_id });
        if let Some(turn_id) = expected_turn_id.as_deref() {
            interrupt_payload["expectedTurnId"] = turn_id.into();
        }
        let receipt = self
            .agent_chat_prompts
            .stop(
                &gent_types::Command {
                    receipt_id: receipt_id.clone(),
                    idempotency_key: format!("agent-chat-interrupt:{}", request_id.0),
                    host_epoch,
                    kind: "agentChatInterrupt".into(),
                    payload: interrupt_payload,
                },
                &conversation_id,
            )?
            .ok_or("agent-chat authority is disabled")?;
        let interrupted = if let Some(turn_id) = expected_turn_id.as_deref() {
            ingress.interrupt_turn(
                selection.provider,
                &target.0,
                turn_id,
                Some(crate::ordinary_lifecycle_cadence::InterruptGoalStop {
                    goals: self.goals.clone(),
                    conversation_id: conversation_id.clone(),
                    host_epoch,
                }),
            )
        } else {
            self.stop_goal(&conversation_id, host_epoch);
            ingress.interrupt_run(selection.provider, &target.0)
        };
        if let Err(error) = interrupted {
            if let Some(turn_id) = expected_turn_id.as_deref()
                && let Some(ended) =
                    self.ended_turn(&conversation_id, &[&run_id, &target], turn_id)?
            {
                return Err(ended.into());
            }
            if running.is_none() {
                return Err(self.settle_idle_run(&target)?);
            }
            return Err(error.into());
        }
        Ok(Some(AgentChatIntentFrame::Interrupted {
            request_id,
            receipt,
            conversation_id,
            run_id,
        }))
    }

    fn ended_turn(
        &self,
        conversation_id: &AgentChatConversationId,
        runs: &[&AgentChatRunId],
        turn_id: &str,
    ) -> Result<Option<InterruptTurnEnded>, String> {
        let ledger = &self.transcript_import_ledger;
        for run in runs {
            let Some(turn) = ledger
                .list_run_turns(&run.0)
                .map_err(|error| error.to_string())?
                .into_iter()
                .find(|turn| turn.turn_id == turn_id)
            else {
                continue;
            };
            if !turn.phase.is_terminal() {
                return Ok(None);
            }
            let background_work_running = ledger
                .run_live_status(&run.0)
                .map_err(|error| error.to_string())?
                .is_some_and(|live| {
                    live.status.has_live_subagent_work() || live.status.has_live_command_work()
                });
            return Ok(Some(InterruptTurnEnded {
                conversation_id: conversation_id.0.clone(),
                run_id: run.0.clone(),
                turn_id: turn.turn_id,
                turn_phase: turn.phase,
                background_work_running,
            }));
        }
        Ok(None)
    }

    fn settle_idle_run(&self, run_id: &AgentChatRunId) -> Result<AgentChatIntentError, String> {
        self.transcript_import_ledger
            .end_run_background_tools(&run_id.0, ToolPhase::Interrupted)
            .map_err(|error| error.to_string())?;
        Ok(AgentChatRejection::NothingToInterrupt.into())
    }

    fn stop_goal(&self, conversation_id: &AgentChatConversationId, host_epoch: HostEpoch) {
        if let Err(error) =
            self.goals
                .stop(conversation_id, host_epoch, crate::startup::unix_seconds())
        {
            eprintln!(
                "goal stop for conversation {} was not recorded: {error}",
                conversation_id.0
            );
        }
    }
}
