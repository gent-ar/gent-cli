use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, Mutex},
};

use gent_ports::{
    AgentChatProjectionLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger, ConversationLedger,
    PendingPermissionLedger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, DurableTurnPhase, HostEpoch,
    PermissionDecisionResponse, PermissionDecisionResponseKind, ReceiptId, WorkspaceRecord,
};
use serde_json::{Value, json};

use super::{ClaurstAcpBridge, ClaurstAcpStdio, ClaurstBridgeHandle};
use crate::{
    claurst_prompt_lifecycle::ClaurstPromptLifecycle,
    ordinary_lifecycle_cadence::AsyncOrdinaryLifecycleHost,
};

struct Held {
    prompt: Value,
    permission: u64,
    session: String,
}

#[derive(Default)]
struct ClaurstPeer {
    written: Arc<Mutex<Vec<Value>>>,
    outbox: VecDeque<Vec<u8>>,
    sessions: u32,
    cancelled: BTreeSet<String>,
    held: Option<Held>,
}

impl ClaurstPeer {
    fn send(&mut self, value: Value) {
        self.outbox.push_back(serde_json::to_vec(&value).unwrap());
    }

    fn update(&mut self, session_id: &str, update: Value) {
        self.send(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":session_id,"update":update}}));
    }

    fn prompt(&mut self, id: Value, session_id: String, text: &str) {
        if self.cancelled.contains(&session_id) {
            return self.send(json!({"jsonrpc":"2.0","id":id,"result":{"stopReason":"cancelled"}}));
        }
        if text.trim_end().ends_with("Please use EnterWorktree") {
            self.update(&session_id, json!({"sessionUpdate":"tool_call","toolCallId":"tool-1","title":"EnterWorktree","kind":"other","status":"in_progress"}));
            self.send(json!({"jsonrpc":"2.0","id":7,"method":"session/request_permission","params":{"sessionId":session_id,"toolCall":{"toolCallId":"tool-1","title":"EnterWorktree","kind":"other"},"options":[{"kind":"allow_once","name":"Allow once","optionId":"allow_once"}]}}));
            self.held = Some(Held {
                prompt: id,
                permission: 7,
                session: session_id,
            });
            return;
        }
        self.update(
            &session_id,
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"ready"}}),
        );
        self.send(json!({"jsonrpc":"2.0","id":id,"result":{"stopReason":"end_turn"}}));
    }

    fn permission_answered(&mut self, id: u64, outcome: &str) {
        let Some(held) = self.held.take_if(|held| held.permission == id) else {
            return;
        };
        let stop = if outcome == "cancelled" && self.cancelled.contains(&held.session) {
            self.update(
                &held.session,
                json!({"sessionUpdate":"tool_call_update","toolCallId":"tool-1","status":"failed"}),
            );
            "cancelled"
        } else {
            self.update(&held.session, json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"worktree ready"}}));
            "end_turn"
        };
        self.send(json!({"jsonrpc":"2.0","id":held.prompt,"result":{"stopReason":stop}}));
    }
}

impl ClaurstAcpStdio for ClaurstPeer {
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String> {
        let value: Value = serde_json::from_slice(frame).unwrap();
        self.written.lock().unwrap().push(value.clone());
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        match value.get("method").and_then(Value::as_str) {
            Some("initialize") => {
                self.send(json!({"jsonrpc":"2.0","id":id,"result":{"agentCapabilities":{}}}))
            }
            Some("session/new") => {
                self.sessions += 1;
                let session_id = format!("acp-{}", self.sessions);
                self.send(json!({"jsonrpc":"2.0","id":id,"result":{"sessionId":session_id}}));
            }
            Some("session/prompt") => {
                let session_id = value["params"]["sessionId"].as_str().unwrap().to_owned();
                let text = value["params"]["prompt"][0]["text"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                self.prompt(id, session_id, &text);
            }
            Some("session/cancel") => {
                self.cancelled
                    .insert(value["params"]["sessionId"].as_str().unwrap().to_owned());
            }
            _ => {
                let outcome = value["result"]["outcome"]["outcome"]
                    .as_str()
                    .unwrap_or_default();
                self.permission_answered(id.as_u64().unwrap_or_default(), outcome);
            }
        }
        Ok(())
    }

    fn try_read_frame(&mut self, _: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(self.outbox.pop_front())
    }
}

fn prompt(ledger: &SqliteLedger, key: &str, text: &str) -> AgentChatPromptSaved {
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(format!("request-{key}")),
            receipt_id: ReceiptId(format!("receipt-{key}")),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: text.into(),
        })
        .unwrap();
    crate::readiness_test_support::release(ledger, &saved);
    saved
}

async fn drive_until_terminal<H: AsyncOrdinaryLifecycleHost>(
    lifecycle: &mut H,
    ledger: &SqliteLedger,
    saved: &AgentChatPromptSaved,
) -> DurableTurnPhase {
    for _ in 0..16 {
        lifecycle.drive_once().await.unwrap();
        let phase = ledger
            .find_turn(&saved.message.turn_id)
            .unwrap()
            .unwrap()
            .phase;
        if phase.is_terminal() {
            return phase;
        }
    }
    ledger
        .find_turn(&saved.message.turn_id)
        .unwrap()
        .unwrap()
        .phase
}

#[tokio::test]
async fn interrupt_with_a_pending_permission_cancels_it_and_settles_the_turn() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation = AgentChatConversationId("conversation-a".into());
    let run = AgentChatRunId("run-a".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation.clone(),
                run_id: run.clone(),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claurst,
                    model: "qwen3-1-7b-q4-k-m".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
    let peer = ClaurstPeer::default();
    let written = Arc::clone(&peer.written);
    let bridge = ClaurstBridgeHandle::new(Arc::new(ClaurstAcpBridge::new(
        "/workspace-a".into(),
        peer,
        vec![],
    )));
    let mut lifecycle =
        ClaurstPromptLifecycle::new(ledger.clone(), bridge, "gentd-1".into(), HostEpoch(1));
    lifecycle.activate_recovery().await.unwrap();
    let waiting = prompt(&ledger, "waiting", "Please use EnterWorktree");
    for _ in 0..4 {
        lifecycle.drive_once().await.unwrap();
    }
    assert!(
        ledger
            .pending_permission(&conversation, &run)
            .unwrap()
            .is_some()
    );

    lifecycle.interrupt_run(&run.0).await.unwrap();

    assert_eq!(
        drive_until_terminal(&mut lifecycle, &ledger, &waiting).await,
        DurableTurnPhase::Interrupted
    );
    assert!(
        ledger
            .pending_permission(&conversation, &run)
            .unwrap()
            .is_none()
    );
    let written = written.lock().unwrap().clone();
    let cancel = written
        .iter()
        .position(|frame| frame["method"] == "session/cancel")
        .unwrap();
    assert_eq!(written[cancel + 1]["id"], 7);
    assert_eq!(
        written[cancel + 1]["result"]["outcome"]["outcome"],
        "cancelled"
    );
    let projection = ledger
        .agent_chat_projection_page(&conversation, 0, 100)
        .unwrap();
    assert!(projection.events.iter().any(|event| {
        event.payload["activity"]["type"] == "decisionSettled"
            && event.payload["activity"]["decisionId"] == "7"
    }));

    let next = prompt(&ledger, "next", "Please use EnterWorktree");
    for _ in 0..4 {
        lifecycle.drive_once().await.unwrap();
    }
    let reused = ledger
        .pending_permission(&conversation, &run)
        .unwrap()
        .unwrap();
    assert_eq!(reused.binding.decision_id.0, "7");
    assert_eq!(reused.binding.turn_id, next.message.turn_id);
    lifecycle
        .respond_permission_with_receipt(
            PermissionDecisionResponse {
                binding: reused.binding,
                response: PermissionDecisionResponseKind::ApproveOnce,
                input: None,
            },
            ReceiptId("approve-next".into()),
        )
        .await
        .unwrap();
    assert_eq!(
        drive_until_terminal(&mut lifecycle, &ledger, &next).await,
        DurableTurnPhase::Completed
    );
}
