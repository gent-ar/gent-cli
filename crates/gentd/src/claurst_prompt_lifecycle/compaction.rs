use std::sync::Arc;

use gent_drivers::conversation_context_input::{
    HistoryWindow, MAX_FRESH_CONTEXT_INPUT_BYTES, render_fresh_conversation_input,
};
use gent_drivers::conversation_context_summary::{
    MAX_SUMMARY_REQUESTS, SUMMARY_ITEM_OVERHEAD_BYTES, summary_chunk_bytes, summary_chunks,
    summary_request,
};
use gent_ports::{ClaurstSourceId, ClaurstStartRequest, PrivateClaurstBridge};
use gent_runtime::{ContextCompactionBudget, ContextCompactionDecision};
use gent_types::{
    AgentChatPromptSaved, AgentChatRunContext, ContextCompactionFact, ContextCompactionFailure,
    ContextCompactionPlan, ContextCompactionTrigger, DurableTurnPhase,
};
use tokio::task::JoinHandle;

use super::{ActivePrompt, ClaurstLifecycleLedger, ClaurstPromptLifecycle, display};
use crate::claurst_runtime_factory::{
    ClaurstRuntimeFactory, ContextSummarizer, LocalContextWindow,
};

const TAIL_ENTRIES: usize = 64;
const TAIL_TRANSCRIPT_ITEMS: usize = 100;
const SOURCE_ITEM_BYTES: usize = 4 * 1024;

#[derive(Debug)]
pub(super) enum Work {
    Provider,
    Compacting(Compaction),
}

#[derive(Debug)]
pub(super) struct Compaction {
    task: JoinHandle<ContextCompactionFact>,
    request: Option<Box<ClaurstStartRequest>>,
    interrupted: bool,
}

pub(super) enum Admission {
    Reject(String),
    Prompt {
        fallback: bool,
    },
    Compact {
        plan: Box<ContextCompactionPlan>,
        then_start: bool,
    },
}

impl<L, B, F> ClaurstPromptLifecycle<L, B, F>
where
    L: ClaurstLifecycleLedger,
    B: PrivateClaurstBridge,
    F: ClaurstRuntimeFactory,
{
    pub(super) fn admission(
        &self,
        saved: &AgentChatPromptSaved,
        boundary: &AgentChatRunContext,
        request: &ClaurstStartRequest,
        without_attachments: bool,
        summarizer: Option<&Arc<dyn ContextSummarizer>>,
    ) -> Result<Admission, String> {
        let window = summarizer.map(|summarizer| summarizer.window());
        let message = (
            saved.message.message_id.as_str(),
            saved.message.turn_id.as_str(),
        );
        if without_attachments
            && gent_types::slash_command(&saved.message.text) == Some(("compact", ""))
        {
            let decision = self
                .artifacts
                .plan_run_compaction(
                    boundary,
                    message,
                    ContextCompactionTrigger::Command,
                    budget(window.unwrap_or(LocalContextWindow {
                        history_input_bytes: MAX_FRESH_CONTEXT_INPUT_BYTES,
                        summary_input_bytes: MAX_FRESH_CONTEXT_INPUT_BYTES,
                    })),
                )
                .map_err(display)?;
            return match decision {
                ContextCompactionDecision::Plan(plan) => Ok(Admission::Compact {
                    plan,
                    then_start: false,
                }),
                ContextCompactionDecision::NothingToCover
                | ContextCompactionDecision::BackingOff => Ok(Admission::Reject(
                    "this conversation has no earlier history to compact".into(),
                )),
            };
        }
        let Some(window) = window else {
            return Ok(Admission::Prompt { fallback: false });
        };
        let truncated = render_fresh_conversation_input(
            &request.context,
            &request.prompt,
            window.history_input_bytes,
        )
        .is_ok_and(|input| input.window() == HistoryWindow::Truncated);
        if !truncated {
            return Ok(Admission::Prompt { fallback: false });
        }
        match self
            .artifacts
            .plan_run_compaction(
                boundary,
                message,
                ContextCompactionTrigger::Budget,
                budget(window),
            )
            .map_err(display)?
        {
            ContextCompactionDecision::Plan(plan) => Ok(Admission::Compact {
                plan,
                then_start: true,
            }),
            ContextCompactionDecision::NothingToCover | ContextCompactionDecision::BackingOff => {
                Ok(Admission::Prompt { fallback: true })
            }
        }
    }

    pub(super) async fn begin_compaction(
        &mut self,
        saved: AgentChatPromptSaved,
        source_id: ClaurstSourceId,
        plan: ContextCompactionPlan,
        request: Option<Box<ClaurstStartRequest>>,
        summarizer: Option<Arc<dyn ContextSummarizer>>,
    ) -> Result<bool, String> {
        let message_id = saved.message.message_id.clone();
        if let Err(error) = self
            .dispatches
            .begin_launch(&message_id, &self.coordinator_id, self.host_epoch)
            .and_then(|()| {
                self.dispatches
                    .confirm_started(&message_id, &self.coordinator_id, self.host_epoch)
            })
        {
            return Err(self.stop_failed_runtime(&saved, display(error)).await);
        }
        self.record_turn_started(&saved, &source_id)?;
        let task = tokio::spawn(summarize(summarizer, plan));
        self.active.insert(
            source_id,
            ActivePrompt {
                saved,
                work: Work::Compacting(Compaction {
                    task,
                    request,
                    interrupted: false,
                }),
            },
        );
        Ok(true)
    }

    pub(super) fn interrupt_compaction(&mut self, run_id: &str) -> bool {
        self.active
            .values_mut()
            .any(|active| match &mut active.work {
                Work::Compacting(compaction) if active.saved.run_id.0 == run_id => {
                    compaction.task.abort();
                    compaction.interrupted = true;
                    true
                }
                _ => false,
            })
    }

    pub(super) async fn drain_compaction(
        &mut self,
        source_id: &ClaurstSourceId,
    ) -> Result<bool, String> {
        let finished = self
            .active
            .get(source_id)
            .is_some_and(|active| match &active.work {
                Work::Compacting(compaction) => compaction.task.is_finished(),
                Work::Provider => false,
            });
        if !finished {
            return Ok(true);
        }
        let Some(ActivePrompt {
            saved,
            work: Work::Compacting(compaction),
        }) = self.active.remove(source_id)
        else {
            return Err("finished Claurst compaction is no longer active".into());
        };
        let fact = match compaction.task.await {
            Ok(fact) if !compaction.interrupted => fact,
            _ => {
                self.settle(&saved, DurableTurnPhase::Interrupted)?;
                return Ok(false);
            }
        };
        self.ledger
            .record_context_compaction(&fact, self.host_epoch)
            .map_err(display)?;
        let Some(mut request) = compaction.request else {
            return match fact {
                ContextCompactionFact::Compacted { .. } => {
                    self.settle(&saved, DurableTurnPhase::Completed)?;
                    self.runtime
                        .after_prompt_settled(&saved.message.conversation_id)
                        .await
                        .map(|()| false)
                }
                ContextCompactionFact::Failed { .. } => self
                    .settle(&saved, DurableTurnPhase::Failed)
                    .map(|()| false),
            };
        };
        request.context = self.prompt_context(&saved)?.0;
        if let Err(error) = self.ingress.start(*request, self.host_epoch).await {
            let error = display(error);
            let failure = self.stop_failed_runtime(&saved, error.clone()).await;
            self.record_notice(
                &saved,
                "claurst-start-failed",
                format!("Claurst could not start: {error}"),
            )?;
            self.settle(&saved, DurableTurnPhase::Failed)?;
            eprintln!("Claurst prompt could not start after compaction: {failure}");
            return Ok(false);
        }
        self.active.insert(
            source_id.clone(),
            ActivePrompt {
                saved,
                work: Work::Provider,
            },
        );
        Ok(true)
    }

    fn settle(&self, saved: &AgentChatPromptSaved, phase: DurableTurnPhase) -> Result<(), String> {
        self.dispatches
            .settle_terminal(
                &saved.message.message_id,
                &self.coordinator_id,
                self.host_epoch,
                phase,
            )
            .map_err(display)
    }
}

fn budget(window: LocalContextWindow) -> ContextCompactionBudget {
    let chunk = summary_chunk_bytes(window.summary_input_bytes);
    ContextCompactionBudget {
        tail_bytes: window.history_input_bytes * 2 / 5,
        tail_entries: TAIL_ENTRIES,
        tail_transcript_items: TAIL_TRANSCRIPT_ITEMS,
        source_bytes: MAX_SUMMARY_REQUESTS
            * chunk.saturating_sub(SOURCE_ITEM_BYTES + SUMMARY_ITEM_OVERHEAD_BYTES),
        item_bytes: SOURCE_ITEM_BYTES,
    }
}

async fn summarize(
    summarizer: Option<Arc<dyn ContextSummarizer>>,
    plan: ContextCompactionPlan,
) -> ContextCompactionFact {
    let Some(summarizer) = summarizer else {
        return plan.failed(ContextCompactionFailure::RuntimeUnavailable);
    };
    let chunks = summary_chunks(
        &plan.items,
        summary_chunk_bytes(summarizer.window().summary_input_bytes),
    );
    let mut summary = plan.previous_summary.clone();
    let mut tokens = 0;
    for chunk in &chunks {
        match summarizer
            .summarize(summary_request(summary.as_deref(), chunk))
            .await
        {
            Ok(output) => {
                tokens = output.tokens;
                summary = Some(output.text);
            }
            Err(failure) => return plan.failed(failure),
        }
    }
    match summary {
        Some(summary) => plan.compacted(summary, tokens),
        None => plan.failed(ContextCompactionFailure::EmptySummary),
    }
}
