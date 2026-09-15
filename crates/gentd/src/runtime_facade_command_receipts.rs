use gent_ports::Ledger;
use gent_protocol::{
    AgentChatIntentFrame, GoalFrame, GoalRejectionCode, agent_chat_commands::CommandOutcome,
};
use gent_types::{
    AgentChatCommandIntent as Intent, AgentChatConversationDetail, AgentChatConversationId, Event,
    HostEpoch, Receipt, ReceiptId,
};
use serde_json::json;

use super::{Resolved, RuntimeFacade};
use crate::agent_chat_intent_error::AgentChatIntentError;

impl RuntimeFacade {
    pub(super) fn recorded_command(
        &self,
        event_id: &str,
        invocation: &serde_json::Value,
    ) -> Result<Option<Resolved>, AgentChatIntentError> {
        let Some(event) = self
            .transcript_import_ledger
            .find_event(event_id)
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        if event.payload["invocation"] != *invocation {
            return Err("agent-chat command receipt is owned by another invocation".into());
        }
        serde_json::from_value(event.payload["resolved"].clone())
            .map(Some)
            .map_err(|error| error.to_string().into())
    }

    pub(super) fn record_command(
        &self,
        event_id: String,
        receipt_id: &ReceiptId,
        host_epoch: HostEpoch,
        invocation: serde_json::Value,
        resolved: Resolved,
    ) -> Result<Resolved, AgentChatIntentError> {
        let payload = json!({"invocation": invocation, "resolved": resolved});
        let recorded = self.transcript_import_ledger.append_event(&Event {
            cursor: 0,
            event_id: event_id.clone(),
            receipt_id: receipt_id.clone(),
            host_epoch,
            kind: "agentChatCommandResolved".into(),
            payload,
        });
        match recorded {
            Ok(_) => Ok(resolved),
            Err(error) => self
                .recorded_command(&event_id, &invocation)?
                .ok_or_else(|| error.to_string().into()),
        }
    }

    pub(super) fn apply_command(
        &self,
        host_epoch: HostEpoch,
        receipt_id: &ReceiptId,
        resolved: Resolved,
    ) -> Result<(gent_types::Receipt, CommandOutcome), AgentChatIntentError> {
        match resolved {
            Resolved::Native { frame } => {
                match self.exchange_agent_chat_intent(frame)?.as_slice() {
                    [
                        AgentChatIntentFrame::Accepted {
                            receipt,
                            run_id,
                            turn_id,
                            message_id,
                            delivery,
                            ..
                        },
                    ] => Ok((
                        receipt.clone(),
                        CommandOutcome::Delivered {
                            run_id: run_id.clone(),
                            turn_id: turn_id.clone(),
                            message_id: message_id.clone(),
                            delivery: *delivery,
                        },
                    )),
                    _ => Err("the provider command was not accepted".into()),
                }
            }
            Resolved::Intent { intent, frame } => {
                applied(intent, self.exchange_agent_chat_intent(frame)?)
            }
            Resolved::Goal {
                conversation_id,
                frame,
            } => goal(self, host_epoch, receipt_id, conversation_id, frame),
        }
    }
}

fn goal(
    facade: &RuntimeFacade,
    host_epoch: HostEpoch,
    receipt_id: &ReceiptId,
    conversation_id: AgentChatConversationId,
    frame: GoalFrame,
) -> Result<(Receipt, CommandOutcome), AgentChatIntentError> {
    let settled = gent_ports::Ledger::find_event(
        &facade.transcript_import_ledger,
        &format!("{}:terminal", receipt_id.0),
    )
    .map_err(|error| error.to_string())?
    .is_some();
    if !settled {
        if let GoalFrame::Rejected { code, .. } =
            crate::goal_api::exchange(&facade.goals, host_epoch, frame)?
        {
            return Err(goal_rejection(code));
        }
    }
    let receipt = facade
        .coordinator
        .submit(&gent_types::Command {
            receipt_id: receipt_id.clone(),
            idempotency_key: format!("agent-chat-command:{}", receipt_id.0),
            host_epoch,
            kind: "agentChatGoalCommand".into(),
            payload: serde_json::json!({ "conversationId": conversation_id }),
        })
        .map_err(|error| error.to_string())?;
    Ok((
        receipt,
        CommandOutcome::IntentApplied {
            intent: Intent::Goal,
            conversation_id,
            run_id: None,
        },
    ))
}

fn goal_rejection(code: GoalRejectionCode) -> AgentChatIntentError {
    AgentChatIntentError {
        code: "goalRejected",
        message: serde_json::to_value(code)
            .ok()
            .and_then(|code| code.as_str().map(str::to_owned))
            .unwrap_or_else(|| "the goal command was rejected".into()),
    }
}

fn applied(
    intent: Intent,
    replies: Vec<AgentChatIntentFrame>,
) -> Result<(Receipt, CommandOutcome), AgentChatIntentError> {
    let Some(
        AgentChatIntentFrame::Created {
            receipt,
            conversation_id,
            run_id,
            ..
        }
        | AgentChatIntentFrame::Switched {
            receipt,
            conversation_id,
            run_id,
            ..
        }
        | AgentChatIntentFrame::Forked {
            receipt,
            conversation_id,
            run_id,
            ..
        },
    ) = replies.into_iter().next()
    else {
        return Err("the command intent was not applied".into());
    };
    Ok((
        receipt,
        CommandOutcome::IntentApplied {
            intent,
            conversation_id,
            run_id: Some(run_id),
        },
    ))
}

pub(super) fn goal_frame(
    facade: &RuntimeFacade,
    detail: &AgentChatConversationDetail,
    receipt_id: &ReceiptId,
    arguments: &str,
) -> Result<Resolved, &'static str> {
    let conversation_id = super::intents::conversation(detail);
    let request_id = receipt_id.0.clone();
    let control = match arguments.to_ascii_lowercase().as_str() {
        "" => return Err("an objective, or pause, resume, or clear"),
        "pause" | "resume" | "clear" => arguments.to_ascii_lowercase(),
        _ => {
            let (token_budget, objective) = goal_budget(arguments)?;
            return Ok(Resolved::Goal {
                frame: GoalFrame::Set {
                    request_id,
                    conversation_id: conversation_id.clone(),
                    objective: objective.to_owned(),
                    token_budget,
                },
                conversation_id,
            });
        }
    };
    let Ok(gent_runtime::GoalResult::Goal(Some(goal))) = facade.goals.current(&conversation_id)
    else {
        return Err("an objective, because this conversation has no goal");
    };
    let (goal_id, expected_revision) = (goal.binding.goal_id, goal.revision);
    let frame = match control.as_str() {
        "pause" => GoalFrame::Pause {
            request_id,
            conversation_id: conversation_id.clone(),
            goal_id,
            expected_revision,
        },
        "resume" => GoalFrame::Resume {
            request_id,
            conversation_id: conversation_id.clone(),
            goal_id,
            expected_revision,
        },
        _ => GoalFrame::Clear {
            request_id,
            conversation_id: conversation_id.clone(),
            goal_id,
            expected_revision,
        },
    };
    Ok(Resolved::Goal {
        conversation_id,
        frame,
    })
}

fn goal_budget(arguments: &str) -> Result<(Option<u64>, &str), &'static str> {
    const HINT: &str = "--budget <tokens, like 200k or 1.5m> followed by an objective";
    let Some(rest) = arguments.strip_prefix("--budget") else {
        return Ok((None, arguments));
    };
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
    let (amount, objective) = rest.split_once(char::is_whitespace).ok_or(HINT)?;
    let (number, multiplier) = match amount.chars().last().map(|unit| unit.to_ascii_lowercase()) {
        Some('k') => (&amount[..amount.len() - 1], 1_000_u128),
        Some('m') => (&amount[..amount.len() - 1], 1_000_000_u128),
        _ => (amount, 1),
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let digits = |part: &str| part.chars().all(|digit| digit.is_ascii_digit());
    if whole.is_empty() || !digits(whole) || !digits(fraction) || fraction.len() > 6 {
        return Err(HINT);
    }
    let scale = 10_u128.pow(u32::try_from(fraction.len()).map_err(|_| HINT)?);
    let scaled = format!("{whole}{fraction}")
        .parse::<u128>()
        .map_err(|_| HINT)?
        * multiplier;
    let tokens = u64::try_from(scaled / scale).map_err(|_| HINT)?;
    let objective = objective.trim();
    if scaled % scale != 0 || tokens == 0 || tokens > 9_007_199_254_740_991 || objective.is_empty()
    {
        return Err(HINT);
    }
    Ok((Some(tokens), objective))
}
