use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, Mutex},
};

use gent_ports::{
    AgentChatProjectionLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger, ConversationLedger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, DurableTurnPhase, HostEpoch, ReceiptId,
    WorkspaceRecord,
};
use serde_json::{Value, json};

use super::{ClaurstAcpBridge, ClaurstAcpStdio, ClaurstBridgeHandle};
use crate::{
    claurst_prompt_lifecycle::ClaurstPromptLifecycle,
    ordinary_lifecycle_cadence::AsyncOrdinaryLifecycleHost,
};

const PING: &str = "toolu_ping";

#[derive(Default)]
struct Claurst {
    written: Vec<Value>,
    outbox: VecDeque<Vec<u8>>,
    sessions: u32,
    cancelled: BTreeSet<String>,
    running: Option<String>,
    end_during_next_turn: bool,
    ignore_cancel: bool,
}

impl Claurst {
    fn update(&mut self, session_id: &str, update: Value) {
        let frame = json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":session_id,"update":update}});
        self.outbox.push_back(serde_json::to_vec(&frame).unwrap());
    }

    fn answer(&mut self, id: &Value, stop: &str) {
        let frame = json!({"jsonrpc":"2.0","id":id,"result":{"stopReason":stop}});
        self.outbox.push_back(serde_json::to_vec(&frame).unwrap());
    }

    fn end_ping(&mut self, raw_status: &str) {
        let status = if raw_status == "completed" {
            "completed"
        } else {
            "failed"
        };
        if let Some(session) = self.running.take() {
            self.update(
                &session,
                json!({"sessionUpdate":"tool_call_update","toolCallId":PING,"status":status,"rawOutput":{"status":raw_status}}),
            );
        }
    }

    fn prompt(&mut self, id: &Value, session: String, text: &str) {
        if self.cancelled.contains(&session) {
            return self.answer(id, "cancelled");
        }
        if text.trim_end().ends_with("ping in the background") {
            self.update(
                &session,
                json!({"sessionUpdate":"tool_call","toolCallId":PING,"title":"Bash","kind":"execute","status":"in_progress","rawInput":{"command":"ping -n 40 127.0.0.1","run_in_background":true}}),
            );
            self.update(
                &session,
                json!({"sessionUpdate":"tool_call_update","toolCallId":PING,"status":"in_progress","rawOutput":"Command started in background."}),
            );
            self.running = Some(session.clone());
        } else if self.end_during_next_turn {
            self.end_ping("completed");
        }
        self.update(
            &session,
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"ready"}}),
        );
        self.answer(id, "end_turn");
    }
}

#[derive(Clone, Default)]
struct Peer(Arc<Mutex<Claurst>>);

impl ClaurstAcpStdio for Peer {
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String> {
        let value: Value = serde_json::from_slice(frame).unwrap();
        let mut claurst = self.0.lock().unwrap();
        claurst.written.push(value.clone());
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        match value.get("method").and_then(Value::as_str) {
            Some("initialize") => {
                let frame = json!({"jsonrpc":"2.0","id":id,"result":{"agentCapabilities":{},"_meta":{"gentPermissionBinding":1}}});
                claurst
                    .outbox
                    .push_back(serde_json::to_vec(&frame).unwrap());
            }
            Some("session/new") => {
                claurst.sessions += 1;
                let frame = json!({"jsonrpc":"2.0","id":id,"result":{"sessionId":format!("acp-{}", claurst.sessions)}});
                claurst
                    .outbox
                    .push_back(serde_json::to_vec(&frame).unwrap());
            }
            Some("session/prompt") => {
                let session = value["params"]["sessionId"].as_str().unwrap().to_owned();
                let text = value["params"]["prompt"][0]["text"].as_str().unwrap();
                claurst.prompt(&id, session, text);
            }
            Some("session/cancel") => {
                let session = value["params"]["sessionId"].as_str().unwrap().to_owned();
                if !claurst.ignore_cancel && claurst.running.as_ref() == Some(&session) {
                    claurst.end_ping("cancelled");
                }
                claurst.cancelled.insert(session);
            }
            _ => {}
        }
        Ok(())
    }

    fn try_read_frame(&mut self, _: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(self.0.lock().unwrap().outbox.pop_front())
    }
}

struct Harness {
    ledger: SqliteLedger,
    claurst: Peer,
    lifecycle: ClaurstPromptLifecycle<SqliteLedger, ClaurstBridgeHandle<Peer>, Releasing>,
    conversation: AgentChatConversationId,
    run: AgentChatRunId,
}

async fn harness() -> Harness {
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
                canonical_path: gent_testkit::host_absolute_path("/workspace-a")
                    .display()
                    .to_string(),
            },
        )
        .unwrap();
    let claurst = Peer::default();
    let bridge = ClaurstBridgeHandle::new(Arc::new(ClaurstAcpBridge::new(
        gent_testkit::host_absolute_path("/workspace-a"),
        claurst.clone(),
        vec![],
    )));
    bridge
        .bind_turn_policy(
            crate::permission_workspace::policy_for_conversation(
                &ledger,
                &conversation.0,
                "workspace-a",
            )
            .unwrap(),
        )
        .unwrap();
    let mut lifecycle = ClaurstPromptLifecycle::new_with_runtime(
        ledger.clone(),
        bridge,
        Releasing(ledger.clone()),
        "gentd-1".into(),
        HostEpoch(1),
    );
    lifecycle.activate_recovery().await.unwrap();
    Harness {
        ledger,
        claurst,
        lifecycle,
        conversation,
        run,
    }
}

impl Harness {
    async fn turn(&mut self, key: &str, text: &str) -> (AgentChatPromptSaved, DurableTurnPhase) {
        let saved = self
            .ledger
            .save_agent_chat_prompt(&AgentChatPromptCreate {
                request_id: AgentChatRequestId(format!("request-{key}")),
                receipt_id: ReceiptId(format!("receipt-{key}")),
                host_epoch: HostEpoch(1),
                conversation_id: self.conversation.clone(),
                disposition: AgentChatPromptDisposition::Send,
                attachment_ids: vec![],
                tool_source_ids: vec![],
                text: text.into(),
            })
            .unwrap();
        crate::readiness_test_support::release(&self.ledger, &saved);
        for _ in 0..16 {
            self.lifecycle.drive_once().await.unwrap();
            let phase = self.phase(&saved);
            if phase.is_terminal() {
                return (saved, phase);
            }
        }
        let phase = self.phase(&saved);
        (saved, phase)
    }

    fn phase(&self, saved: &AgentChatPromptSaved) -> DurableTurnPhase {
        self.ledger
            .find_turn(&saved.message.turn_id)
            .unwrap()
            .unwrap()
            .phase
    }

    fn ping_phases(&self) -> Vec<PingPhase> {
        ping_phases(&self.ledger, &self.conversation)
    }

    fn written(&self, method: &str) -> Vec<Value> {
        self.claurst
            .0
            .lock()
            .unwrap()
            .written
            .iter()
            .filter(|frame| frame["method"] == method)
            .cloned()
            .collect()
    }
}

type PingPhase = (String, String, String);

fn ping_phases(ledger: &SqliteLedger, conversation: &AgentChatConversationId) -> Vec<PingPhase> {
    ledger
        .agent_chat_projection_page(conversation, 0, 100)
        .unwrap()
        .events
        .iter()
        .filter(|event| {
            event.payload["activity"]["type"] == "toolActivity"
                && event.payload["activity"]["activity"]["toolUseId"] == PING
        })
        .map(|event| {
            let activity = &event.payload["activity"];
            (
                activity["runId"].as_str().unwrap().to_owned(),
                activity["turnId"].as_str().unwrap().to_owned(),
                activity["activity"]["phase"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn phases(turn: &AgentChatPromptSaved, phases: &[&str]) -> Vec<PingPhase> {
    phases
        .iter()
        .map(|phase| {
            (
                turn.run_id.0.clone(),
                turn.message.turn_id.clone(),
                (*phase).to_owned(),
            )
        })
        .collect()
}

#[derive(Debug)]
struct Releasing(SqliteLedger);

#[async_trait::async_trait]
impl crate::claurst_runtime_factory::ClaurstRuntimeFactory for Releasing {
    async fn ensure_for_prompt(
        &self,
        _: &AgentChatPromptSaved,
        _: &[String],
    ) -> Result<(), String> {
        Ok(())
    }

    async fn after_prompt_failed(
        &self,
        conversation_id: &str,
        background: gent_types::ToolPhase,
    ) -> Result<(), String> {
        gent_ports::RunLifecycleFactLedger::end_background_tools(
            &self.0,
            conversation_id,
            background,
        )
        .map_err(|error| error.to_string())
    }
}

#[tokio::test]
async fn a_background_command_outlives_its_turn_and_completes_under_it() {
    let mut harness = harness().await;
    let (launch, phase) = harness.turn("launch", "ping in the background").await;
    assert_eq!(phase, DurableTurnPhase::Completed);
    assert_eq!(
        harness.ping_phases(),
        phases(&launch, &["started", "started", "background"])
    );

    harness.claurst.0.lock().unwrap().end_ping("completed");
    gent_drivers::provider_activity::signal();
    assert!(harness.lifecycle.drive_once().await.unwrap());

    assert_eq!(
        harness.ping_phases(),
        phases(&launch, &["started", "started", "background", "completed"])
    );
    let (_, next) = harness.turn("next", "hello").await;
    assert_eq!(next, DurableTurnPhase::Completed);
}

#[tokio::test]
async fn a_background_end_reported_during_the_next_turn_belongs_to_the_launching_turn() {
    let mut harness = harness().await;
    let (launch, _) = harness.turn("launch", "ping in the background").await;
    harness.claurst.0.lock().unwrap().end_during_next_turn = true;
    let (_, next) = harness.turn("next", "hello").await;
    assert_eq!(next, DurableTurnPhase::Completed);
    assert_eq!(
        harness.ping_phases(),
        phases(&launch, &["started", "started", "background", "completed"])
    );
}

#[tokio::test]
async fn stop_with_only_background_work_cancels_it_and_the_next_prompt_gets_a_fresh_session() {
    let mut harness = harness().await;
    let (launch, _) = harness.turn("launch", "ping in the background").await;

    harness
        .lifecycle
        .interrupt_run(&harness.run.0)
        .await
        .unwrap();
    assert_eq!(
        harness.written("session/cancel")[0]["params"]["sessionId"],
        "acp-1"
    );
    assert!(harness.lifecycle.drive_once().await.unwrap());
    assert_eq!(
        harness.ping_phases(),
        phases(
            &launch,
            &["started", "started", "background", "interrupted"]
        )
    );

    let (_, next) = harness.turn("next", "hello").await;
    assert_eq!(next, DurableTurnPhase::Completed);
    let prompts = harness.written("session/prompt");
    assert_eq!(prompts.last().unwrap()["params"]["sessionId"], "acp-2");
    assert_eq!(
        harness
            .lifecycle
            .interrupt_run(&harness.run.0)
            .await
            .unwrap_err(),
        "the interrupt target turn is no longer active"
    );
}

struct Scripted {
    reads: VecDeque<Vec<u8>>,
    written: Arc<Mutex<Vec<Value>>>,
}

impl ClaurstAcpStdio for Scripted {
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String> {
        self.written
            .lock()
            .unwrap()
            .push(serde_json::from_slice(frame).unwrap());
        Ok(())
    }
    fn try_read_frame(&mut self, _: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(self.reads.pop_front())
    }
}

fn start(run: &str) -> gent_ports::ClaurstStartRequest {
    gent_ports::ClaurstStartRequest {
        run_id: run.into(),
        source_id: gent_ports::ClaurstSourceId(format!("source-{run}")),
        turn_id: format!("turn-{run}"),
        prompt: "hello".into(),
        context: gent_types::FrozenConversationContext::cleared(AgentChatConversationId(
            "c-1".into(),
        )),
        attachments: vec![],
        goal: None,
    }
}

async fn settle(bridge: &ClaurstAcpBridge<Scripted>, binding: &gent_ports::ClaurstSessionBinding) {
    use gent_ports::PrivateClaurstBridge;
    let mut cursor = 0;
    loop {
        let batch = bridge
            .drain(gent_ports::ClaurstDrainRequest {
                run_id: binding.run_id.clone(),
                source_id: binding.source_id.clone(),
                after_cursor: cursor,
                limit: 64,
            })
            .await
            .unwrap();
        cursor = batch.checkpoint.unwrap().cursor;
        if batch.terminal.is_some() {
            return;
        }
    }
}

#[tokio::test]
async fn stop_cancels_only_the_background_sessions_of_the_run_it_stops() {
    use gent_ports::PrivateClaurstBridge;
    let frame = |value: Value| serde_json::to_vec(&value).unwrap();
    let update = |session: &str, update: Value| {
        frame(
            json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":session,"update":update}}),
        )
    };
    let written = Arc::new(Mutex::new(Vec::new()));
    let bridge = ClaurstAcpBridge::new(
        gent_testkit::host_absolute_path("/workspace"),
        Scripted {
            written: Arc::clone(&written),
            reads: VecDeque::from([
                frame(
                    json!({"jsonrpc":"2.0","id":1,"result":{"_meta":{"gentPermissionBinding":1}}}),
                ),
                frame(json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":"acp-1"}})),
                update(
                    "acp-1",
                    json!({"sessionUpdate":"tool_call","toolCallId":PING,"title":"Bash","status":"in_progress","rawInput":{}}),
                ),
                update(
                    "acp-1",
                    json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"started"}}),
                ),
                frame(json!({"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}})),
                frame(json!({"jsonrpc":"2.0","id":4,"result":{"sessionId":"acp-2"}})),
                update(
                    "acp-2",
                    json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"ready"}}),
                ),
                frame(json!({"jsonrpc":"2.0","id":5,"result":{"stopReason":"end_turn"}})),
            ]),
        },
        vec![],
    );
    let first = bridge.start(start("run-1")).await.unwrap();
    settle(&bridge, &first).await;
    let second = bridge.start(start("run-2")).await.unwrap();
    assert_eq!(second.opaque_session_id, "acp-2");
    settle(&bridge, &second).await;
    let cancels = || {
        written
            .lock()
            .unwrap()
            .iter()
            .filter(|frame| frame["method"] == "session/cancel")
            .map(|frame| frame["params"]["sessionId"].clone())
            .collect::<Vec<_>>()
    };

    assert!(bridge.cancel(second).await.is_err());
    assert!(cancels().is_empty());
    bridge.cancel(first).await.unwrap();
    assert_eq!(cancels(), [json!("acp-1")]);
}

struct PerConversation(std::collections::BTreeMap<String, ClaurstBridgeHandle<Peer>>);

impl PerConversation {
    fn route(&self, run: &str) -> Result<&ClaurstBridgeHandle<Peer>, gent_ports::PortError> {
        self.0
            .get(run)
            .ok_or_else(|| gent_ports::PortError::Unavailable(run.into()))
    }
}

#[async_trait::async_trait]
impl gent_ports::PrivateClaurstBridge for PerConversation {
    async fn start(
        &self,
        request: gent_ports::ClaurstStartRequest,
    ) -> Result<gent_ports::ClaurstSessionBinding, gent_ports::PortError> {
        self.route(&request.run_id)?.start(request).await
    }
    async fn bind_session(
        &self,
        binding: gent_ports::ClaurstSessionBinding,
    ) -> Result<(), gent_ports::PortError> {
        self.route(&binding.run_id)?.bind_session(binding).await
    }
    async fn cancel(
        &self,
        binding: gent_ports::ClaurstSessionBinding,
    ) -> Result<(), gent_ports::PortError> {
        self.route(&binding.run_id)?.cancel(binding).await
    }
    async fn drain(
        &self,
        request: gent_ports::ClaurstDrainRequest,
    ) -> Result<gent_ports::ClaurstDrainBatch, gent_ports::PortError> {
        self.route(&request.run_id)?.drain(request).await
    }
    async fn permission_authority(
        &self,
        binding: gent_ports::ClaurstSessionBinding,
        epoch: HostEpoch,
    ) -> Result<(gent_types::PolicyRecord, gent_types::SandboxEnforcement), gent_ports::PortError>
    {
        self.route(&binding.run_id)?
            .permission_authority(binding, epoch)
            .await
    }
    async fn runs_background_commands(&self, binding: &gent_ports::ClaurstSessionBinding) -> bool {
        match self.route(&binding.run_id) {
            Ok(bridge) => bridge.runs_background_commands(binding).await,
            Err(_) => false,
        }
    }
}

impl std::fmt::Debug for PerConversation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PerConversation(..)")
    }
}

#[tokio::test]
async fn stopping_one_conversation_leaves_another_conversations_background_command_running() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let mut bridges = std::collections::BTreeMap::new();
    let mut peers = Vec::new();
    for name in ["a", "b"] {
        let conversation = AgentChatConversationId(format!("conversation-{name}"));
        ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId(format!("conversation-receipt-{name}")),
                    idempotency_key: format!("conversation-key-{name}"),
                    host_epoch: HostEpoch(1),
                    conversation_id: conversation.clone(),
                    run_id: AgentChatRunId(format!("run-{name}")),
                    selection: AgentChatSelection {
                        provider: AgentChatProvider::Claurst,
                        model: "qwen3-1-7b-q4-k-m".into(),
                        effort: AgentChatEffort::Medium,
                        mode: AgentChatMode::Agent,
                    },
                },
                &WorkspaceRecord {
                    workspace_id: "workspace-a".into(),
                    canonical_path: gent_testkit::host_absolute_path("/workspace-a")
                        .display()
                        .to_string(),
                },
            )
            .unwrap();
        let peer = Peer::default();
        let bridge = ClaurstBridgeHandle::new(Arc::new(ClaurstAcpBridge::new(
            gent_testkit::host_absolute_path("/workspace-a"),
            peer.clone(),
            vec![],
        )));
        bridge
            .bind_turn_policy(
                crate::permission_workspace::policy_for_conversation(
                    &ledger,
                    &conversation.0,
                    "workspace-a",
                )
                .unwrap(),
            )
            .unwrap();
        bridges.insert(format!("run-{name}"), bridge);
        peers.push(peer);
    }
    let mut lifecycle = ClaurstPromptLifecycle::new(
        ledger.clone(),
        PerConversation(bridges),
        "gentd-1".into(),
        HostEpoch(1),
    );
    lifecycle.activate_recovery().await.unwrap();
    for name in ["a", "b"] {
        let saved = ledger
            .save_agent_chat_prompt(&AgentChatPromptCreate {
                request_id: AgentChatRequestId(format!("request-{name}")),
                receipt_id: ReceiptId(format!("receipt-{name}")),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId(format!("conversation-{name}")),
                disposition: AgentChatPromptDisposition::Send,
                attachment_ids: vec![],
                tool_source_ids: vec![],
                text: "ping in the background".into(),
            })
            .unwrap();
        crate::readiness_test_support::release(&ledger, &saved);
        for _ in 0..16 {
            lifecycle.drive_once().await.unwrap();
        }
        assert_eq!(
            ledger
                .find_turn(&saved.message.turn_id)
                .unwrap()
                .unwrap()
                .phase,
            DurableTurnPhase::Completed
        );
    }
    let ping = |name: &str| {
        ping_phases(
            &ledger,
            &AgentChatConversationId(format!("conversation-{name}")),
        )
        .last()
        .map(|(_, _, phase)| phase.clone())
    };
    assert_eq!(
        (ping("a"), ping("b")),
        (Some("background".into()), Some("background".into()))
    );

    lifecycle.interrupt_run("run-a").await.unwrap();
    for _ in 0..4 {
        lifecycle.drive_once().await.unwrap();
    }

    let cancelled = |peer: &Peer| {
        peer.0
            .lock()
            .unwrap()
            .written
            .iter()
            .filter(|frame| frame["method"] == "session/cancel")
            .count()
    };
    assert_eq!((cancelled(&peers[0]), cancelled(&peers[1])), (1, 0));
    assert_eq!(
        (ping("a"), ping("b")),
        (Some("interrupted".into()), Some("background".into()))
    );
    assert!(peers[1].0.lock().unwrap().running.is_some());
}

impl Harness {
    fn command_running(&self, run: &str) -> bool {
        gent_ports::RunLifecycleFactLedger::run_live_status(&self.ledger, run)
            .unwrap()
            .is_some_and(|live| live.status.has_live_command_work())
    }
}

#[tokio::test]
async fn a_runtime_release_ends_background_commands_and_until_then_they_are_live_work() {
    let mut harness = harness().await;
    let (launch, _) = harness.turn("launch", "ping in the background").await;
    assert!(harness.command_running(&harness.run.0));

    gent_ports::RunLifecycleFactLedger::end_background_tools(
        &harness.ledger,
        &harness.conversation.0,
        gent_types::ToolPhase::Failed,
    )
    .unwrap();

    assert_eq!(
        harness.ping_phases(),
        phases(&launch, &["started", "started", "background", "failed"])
    );
    assert!(!harness.command_running(&harness.run.0));
}

#[tokio::test]
async fn a_gentd_restart_fails_background_commands_of_earlier_epochs_with_a_typed_notice() {
    use gent_ports::{AgentChatPromptDispatchLedger, AgentChatReadLedger, Ledger};
    let mut harness = harness().await;
    let (launch, _) = harness.turn("launch", "ping in the background").await;
    harness
        .ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(1))
        .unwrap();
    assert_eq!(
        harness.ping_phases(),
        phases(&launch, &["started", "started", "background"])
    );

    harness.ledger.close_ingress(HostEpoch(1)).unwrap();
    harness.ledger.fence_and_open(HostEpoch(1)).unwrap();
    harness
        .ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(2))
        .unwrap();

    assert_eq!(
        harness.ping_phases(),
        phases(&launch, &["started", "started", "background", "failed"])
    );
    let notices = harness
        .ledger
        .read_agent_chat_transcript(&harness.conversation.0, None, 50)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| {
            event.kind == gent_types::NormalizedTranscriptKind::Notice
                && event.turn_id == launch.message.turn_id
        })
        .collect::<Vec<_>>();
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].failure_code,
        Some(gent_types::AgentChatFailureCode::GentdRestarted)
    );
    assert_eq!(
        notices[0].text,
        "Gentd restarted, which stopped a background command this turn had started."
    );
}

#[tokio::test]
async fn a_stop_that_races_a_completed_answer_still_ends_its_background_commands_interrupted() {
    let mut harness = harness().await;
    harness.claurst.0.lock().unwrap().ignore_cancel = true;
    let saved = harness
        .ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("request-race".into()),
            receipt_id: ReceiptId("receipt-race".into()),
            host_epoch: HostEpoch(1),
            conversation_id: harness.conversation.clone(),
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "ping in the background".into(),
        })
        .unwrap();
    crate::readiness_test_support::release(&harness.ledger, &saved);
    for _ in 0..16 {
        if !harness.written("session/prompt").is_empty() {
            break;
        }
        harness.lifecycle.drive_once().await.unwrap();
    }
    harness
        .lifecycle
        .interrupt_run(&harness.run.0)
        .await
        .unwrap();
    for _ in 0..4 {
        harness.lifecycle.drive_once().await.unwrap();
    }

    assert_eq!(harness.phase(&saved), DurableTurnPhase::Interrupted);
    assert_eq!(
        harness.ping_phases(),
        phases(&saved, &["started", "started", "background", "interrupted"])
    );
}

#[tokio::test]
async fn a_background_end_reported_during_a_later_run_is_filed_under_the_launching_run() {
    use gent_ports::AgentChatSelectionLedger;
    let mut harness = harness().await;
    let (launch, _) = harness.turn("launch", "ping in the background").await;
    harness
        .ledger
        .switch_agent_chat_selection(&gent_types::AgentChatSelectionSwitch {
            receipt_id: ReceiptId("switch".into()),
            idempotency_key: "switch".into(),
            host_epoch: HostEpoch(1),
            conversation_id: harness.conversation.clone(),
            parent_run_id: harness.run.clone(),
            run_id: AgentChatRunId("run-later".into()),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Claurst,
                model: "qwen3-1-7b-q4-k-m".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Agent,
            },
            context_policy: gent_types::ContextPolicy::Preserve,
        })
        .unwrap();
    harness.claurst.0.lock().unwrap().end_during_next_turn = true;

    let (later, phase) = harness.turn("later", "hello").await;

    assert_eq!(phase, DurableTurnPhase::Completed);
    assert_eq!(later.run_id.0, "run-later");
    assert_eq!(
        harness.ping_phases(),
        phases(&launch, &["started", "started", "background", "completed"])
    );
    assert!(!harness.command_running(&harness.run.0));
    assert!(!harness.command_running("run-later"));
}
