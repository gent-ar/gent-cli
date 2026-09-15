use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use gent_drivers::conversation_context_summary::SummaryRequest;
use gent_ports::{
    AgentChatPromptLedger, AgentChatWorkspaceLedger, ClaurstCheckpoint, ClaurstDrainBatch,
    ClaurstSessionBinding, ClaurstSourceId, ContextCompactionLedger, ConversationLedger,
    TranscriptLedger,
};
use gent_store::SqliteLedger;
use gent_testkit::FakePrivateClaurstBridge;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, CONTEXT_COMPACTION_FALLBACK_NOTICE,
    ContextCompactionFact, ContextCompactionFailure, ContextCompactionTrigger, DurableTurnPhase,
    HostEpoch, NormalizedTranscriptAppend, NormalizedTranscriptEvent, NormalizedTranscriptKind,
    PROVIDER_CONTEXT_COMPACTED_NOTICE, PROVIDER_CONTEXT_COMPACTION_FAILED_NOTICE, ReceiptId,
    WorkspaceRecord,
};
use sha2::{Digest, Sha256};

use super::{AsyncOrdinaryLifecycleHost, ClaurstPromptLifecycle};
use crate::claurst_runtime_factory::{
    ClaurstRuntimeFactory, ContextSummarizer, ContextSummary, LocalContextWindow,
    ReadyClaurstRuntime,
};

type Outcome = Result<ContextSummary, ContextCompactionFailure>;

#[derive(Debug)]
struct FakeSummarizer {
    window: LocalContextWindow,
    outcomes: Mutex<VecDeque<Option<Outcome>>>,
    requests: Mutex<Vec<SummaryRequest>>,
}

#[async_trait]
impl ContextSummarizer for FakeSummarizer {
    fn window(&self) -> LocalContextWindow {
        self.window
    }

    async fn summarize(&self, request: SummaryRequest) -> Outcome {
        self.requests.lock().unwrap().push(request);
        let outcome = self.outcomes.lock().unwrap().pop_front().flatten();
        match outcome {
            Some(outcome) => outcome,
            None => std::future::pending().await,
        }
    }
}

#[derive(Debug)]
struct LocalRuntime(Arc<FakeSummarizer>);

#[async_trait]
impl ClaurstRuntimeFactory for LocalRuntime {
    async fn ensure_for_prompt(&self, _: &AgentChatPromptSaved) -> Result<(), String> {
        Ok(())
    }

    async fn context_summarizer(&self) -> Option<Arc<dyn ContextSummarizer>> {
        Some(Arc::clone(&self.0) as Arc<dyn ContextSummarizer>)
    }
}

struct Harness<F: ClaurstRuntimeFactory> {
    ledger: SqliteLedger,
    bridge: FakePrivateClaurstBridge,
    lifecycle: ClaurstPromptLifecycle<SqliteLedger, FakePrivateClaurstBridge, F>,
    prompts: usize,
}

fn summarizer(history_input_bytes: usize, outcomes: Vec<Option<Outcome>>) -> Arc<FakeSummarizer> {
    Arc::new(FakeSummarizer {
        window: LocalContextWindow {
            history_input_bytes,
            summary_input_bytes: 93_696,
        },
        outcomes: Mutex::new(outcomes.into()),
        requests: Mutex::new(Vec::new()),
    })
}

fn summary(text: &str) -> Option<Outcome> {
    Some(Ok(ContextSummary {
        text: text.into(),
        tokens: 12,
    }))
}

impl<F: ClaurstRuntimeFactory> Harness<F> {
    async fn new(runtime: F) -> Self {
        let ledger = SqliteLedger::in_memory().unwrap();
        ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId("conversation".into()),
                    idempotency_key: "conversation".into(),
                    host_epoch: HostEpoch(1),
                    conversation_id: AgentChatConversationId("conversation-a".into()),
                    run_id: AgentChatRunId("run-a".into()),
                    selection: AgentChatSelection {
                        provider: AgentChatProvider::Claurst,
                        model: "qwen3-1-7b-q4-k-m".into(),
                        effort: AgentChatEffort::Low,
                        mode: AgentChatMode::Agent,
                    },
                },
                &WorkspaceRecord {
                    workspace_id: "workspace-a".into(),
                    canonical_path: "/workspace-a".into(),
                },
            )
            .unwrap();
        let bridge = FakePrivateClaurstBridge::default();
        let mut lifecycle = ClaurstPromptLifecycle::new_with_runtime(
            ledger.clone(),
            bridge.clone(),
            runtime,
            "gentd-1".into(),
            HostEpoch(1),
        );
        lifecycle.activate_recovery().await.unwrap();
        Self {
            ledger,
            bridge,
            lifecycle,
            prompts: 0,
        }
    }

    fn save(&mut self, text: &str) -> AgentChatPromptSaved {
        self.prompts += 1;
        let saved = self
            .ledger
            .save_agent_chat_prompt(&AgentChatPromptCreate {
                request_id: AgentChatRequestId(format!("prompt-{}", self.prompts)),
                receipt_id: ReceiptId(format!("prompt-{}", self.prompts)),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation-a".into()),
                disposition: AgentChatPromptDisposition::Send,
                attachment_ids: vec![],
                tool_source_ids: vec![],
                text: text.into(),
            })
            .unwrap();
        crate::readiness_test_support::release(&self.ledger, &saved);
        saved
    }

    fn expect_provider_turn(&self, saved: &AgentChatPromptSaved) {
        let binding = binding(saved);
        self.bridge.push_start_binding(binding.clone());
        self.bridge.push_batch(ClaurstDrainBatch {
            facts: vec![],
            permissions: vec![],
            checkpoint: Some(ClaurstCheckpoint {
                run_id: binding.run_id.clone(),
                source_id: binding.source_id.clone(),
                cursor: 0,
                state_digest_sha256: "d".repeat(64),
            }),
            session_binding: Some(binding),
            terminal: Some(gent_ports::ClaurstTerminal::Completed),
        });
    }

    async fn settle(&mut self, saved: &AgentChatPromptSaved) -> DurableTurnPhase {
        for _ in 0..400 {
            let phase = self.phase(saved);
            if phase.is_terminal() {
                return phase;
            }
            self.lifecycle.drive_once().await.unwrap();
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        panic!("turn {} did not settle", saved.message.turn_id);
    }

    fn phase(&self, saved: &AgentChatPromptSaved) -> DurableTurnPhase {
        self.ledger
            .find_turn(&saved.message.turn_id)
            .unwrap()
            .unwrap()
            .phase
    }

    async fn provider_turn(&mut self, text: &str, reply: &str) -> AgentChatPromptSaved {
        let saved = self.save(text);
        self.expect_provider_turn(&saved);
        assert_eq!(self.settle(&saved).await, DurableTurnPhase::Completed);
        self.ledger
            .append_normalized_transcript(
                &AgentChatConversationId("conversation-a".into()),
                &NormalizedTranscriptAppend {
                    event_id: format!("reply:{}", saved.message.message_id),
                    turn_id: saved.message.turn_id.clone(),
                    run_id: saved.run_id.0.clone(),
                    kind: NormalizedTranscriptKind::AssistantMessage,
                    text: reply.into(),
                    is_partial: false,
                },
            )
            .unwrap();
        saved
    }

    fn transcript(&self) -> Vec<NormalizedTranscriptEvent> {
        self.ledger
            .normalized_transcript_page(&AgentChatConversationId("conversation-a".into()), 0, 100)
            .unwrap()
            .events
    }

    fn notices(&self, saved: &AgentChatPromptSaved) -> Vec<String> {
        self.transcript()
            .into_iter()
            .filter(|event| {
                event.turn_id == saved.message.turn_id
                    && event.kind == NormalizedTranscriptKind::Notice
            })
            .map(|event| event.text)
            .collect()
    }

    fn facts(&self) -> Vec<ContextCompactionFact> {
        self.ledger
            .context_compactions("conversation-a", 16)
            .unwrap()
    }
}

fn binding(saved: &AgentChatPromptSaved) -> ClaurstSessionBinding {
    let material = format!(
        "{}\0{}\0{}",
        saved.run_id.0, saved.message.turn_id, saved.message.message_id
    );
    ClaurstSessionBinding {
        run_id: saved.run_id.0.clone(),
        source_id: ClaurstSourceId(format!("gent-{}", hex::encode(Sha256::digest(material)))),
        opaque_session_id: format!("acp-{}", saved.message.message_id),
    }
}

#[path = "claurst_prompt_lifecycle_compaction_flow_tests.rs"]
mod flows;
