use gent_ports::{AgentChatPromptLedger, AgentChatWorkspaceLedger, ConversationLedger};
use gent_store::SqliteLedger;
use gent_testkit::FakePrivateClaurstBridge;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, DurableTurnPhase, HostEpoch, ReceiptId,
    WorkspaceRecord,
};

use super::{AsyncOrdinaryLifecycleHost, ClaurstPromptLifecycle};
use crate::claurst_runtime_factory::ClaurstRuntimeFactory;

#[derive(Debug)]
struct LaunchingRuntime(std::sync::Arc<tokio::sync::Semaphore>);

#[async_trait::async_trait]
impl ClaurstRuntimeFactory for LaunchingRuntime {
    async fn ensure_for_prompt(
        &self,
        _: &AgentChatPromptSaved,
        _: &[String],
    ) -> Result<(), String> {
        self.0
            .acquire()
            .await
            .map(drop)
            .map_err(|error| error.to_string())
    }

    async fn warm(&self, _: &str, _: &str, _: &[String]) -> Result<(), String> {
        std::future::pending().await
    }
}

async fn launching(
    ledger: &SqliteLedger,
    bridge: &FakePrivateClaurstBridge,
) -> (
    ClaurstPromptLifecycle<SqliteLedger, FakePrivateClaurstBridge, LaunchingRuntime>,
    std::sync::Arc<tokio::sync::Semaphore>,
) {
    let runtime = LaunchingRuntime(std::sync::Arc::new(tokio::sync::Semaphore::new(0)));
    let launch = std::sync::Arc::clone(&runtime.0);
    let mut lifecycle = ClaurstPromptLifecycle::new_with_runtime(
        ledger.clone(),
        bridge.clone(),
        runtime,
        "gentd-1".into(),
        HostEpoch(1),
    );
    lifecycle.activate_recovery().await.unwrap();
    lifecycle.warm_claurst("conversation-a", "run-a");
    let drive = tokio::time::timeout(std::time::Duration::from_secs(5), lifecycle.drive_once());
    assert!(drive.await.unwrap().unwrap());
    assert!(!lifecycle.is_idle());
    (lifecycle, launch)
}

#[tokio::test]
async fn stopping_a_claurst_turn_while_its_runtime_launches_interrupts_it_without_waiting() {
    let (ledger, saved) = waiting_prompt(AgentChatPromptDisposition::Send);
    let bridge = FakePrivateClaurstBridge::default();
    let (mut lifecycle, launch) = launching(&ledger, &bridge).await;
    lifecycle
        .interrupt_claurst_turn(&saved.run_id.0, &saved.message.turn_id)
        .await
        .unwrap();
    assert_eq!(phase(&ledger, &saved), DurableTurnPhase::Interrupted);
    assert!(lifecycle.is_idle());
    launch.add_permits(1);
    tokio::task::yield_now().await;
    assert!(!lifecycle.drive_once().await.unwrap());
    assert!(bridge.starts().is_empty());
}

#[tokio::test]
async fn a_claurst_turn_starts_once_its_runtime_launch_finishes() {
    let (ledger, saved) = waiting_prompt(AgentChatPromptDisposition::Send);
    let bridge = FakePrivateClaurstBridge::default();
    let (mut lifecycle, launch) = launching(&ledger, &bridge).await;
    assert!(!lifecycle.drive_once().await.unwrap());
    assert!(bridge.starts().is_empty());
    launch.add_permits(1);
    tokio::task::yield_now().await;
    assert!(lifecycle.drive_once().await.unwrap());
    assert_eq!(bridge.starts().len(), 1);
    assert_eq!(bridge.starts()[0].turn_id, saved.message.turn_id);
}

fn waiting_prompt(disposition: AgentChatPromptDisposition) -> (SqliteLedger, AgentChatPromptSaved) {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
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
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-request".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "Reply with exactly the word PING".into(),
        })
        .unwrap();
    crate::readiness_test_support::release(&ledger, &saved);
    (ledger, saved)
}

async fn recovered(
    ledger: &SqliteLedger,
    bridge: &FakePrivateClaurstBridge,
) -> ClaurstPromptLifecycle<SqliteLedger, FakePrivateClaurstBridge> {
    let mut lifecycle = ClaurstPromptLifecycle::new(
        ledger.clone(),
        bridge.clone(),
        "gentd-1".into(),
        HostEpoch(1),
    );
    lifecycle.activate_recovery().await.unwrap();
    lifecycle
}

fn phase(ledger: &SqliteLedger, saved: &AgentChatPromptSaved) -> DurableTurnPhase {
    ledger
        .find_turn(&saved.message.turn_id)
        .unwrap()
        .unwrap()
        .phase
}

#[tokio::test]
async fn stopping_a_claurst_turn_still_waiting_for_the_runtime_interrupts_it_before_it_starts() {
    let (ledger, saved) = waiting_prompt(AgentChatPromptDisposition::Send);
    let bridge = FakePrivateClaurstBridge::default();
    let mut lifecycle = recovered(&ledger, &bridge).await;
    lifecycle
        .interrupt_claurst_turn(&saved.run_id.0, &saved.message.turn_id)
        .await
        .unwrap();
    assert_eq!(phase(&ledger, &saved), DurableTurnPhase::Interrupted);
    assert!(!lifecycle.drive_once().await.unwrap());
    assert!(bridge.starts().is_empty());
}

#[tokio::test]
async fn stopping_a_claurst_run_with_nothing_started_interrupts_its_waiting_turn() {
    let (ledger, saved) = waiting_prompt(AgentChatPromptDisposition::Send);
    let bridge = FakePrivateClaurstBridge::default();
    let mut lifecycle = recovered(&ledger, &bridge).await;
    lifecycle
        .interrupt_claurst_run(&saved.run_id.0)
        .await
        .unwrap();
    assert_eq!(phase(&ledger, &saved), DurableTurnPhase::Interrupted);
    assert!(!lifecycle.drive_once().await.unwrap());
    assert!(bridge.starts().is_empty());
}

#[tokio::test]
async fn stopping_a_claurst_run_leaves_its_queued_prompts_to_their_own_cancel() {
    let (ledger, saved) = waiting_prompt(AgentChatPromptDisposition::Queue);
    let bridge = FakePrivateClaurstBridge::default();
    let mut lifecycle = recovered(&ledger, &bridge).await;
    assert!(
        lifecycle
            .interrupt_claurst_run(&saved.run_id.0)
            .await
            .is_err()
    );
    assert_eq!(phase(&ledger, &saved), DurableTurnPhase::Active);
}

#[derive(Debug)]
struct StoppingRuntime(std::sync::Arc<tokio::sync::Semaphore>);

#[async_trait::async_trait]
impl ClaurstRuntimeFactory for StoppingRuntime {
    async fn ensure_for_prompt(
        &self,
        _: &AgentChatPromptSaved,
        _: &[String],
    ) -> Result<(), String> {
        Ok(())
    }

    async fn warm(&self, _: &str, _: &str, _: &[String]) -> Result<(), String> {
        Ok(())
    }

    async fn reap_idle(&self, _: std::time::Instant, _: &[String]) -> Result<(), String> {
        self.0
            .acquire()
            .await
            .map(drop)
            .map_err(|error| error.to_string())
    }
}

#[tokio::test]
async fn reaping_an_idle_runtime_never_holds_the_lifecycle_while_that_runtime_stops() {
    let (ledger, _) = waiting_prompt(AgentChatPromptDisposition::Send);
    let stopping = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    let mut lifecycle = ClaurstPromptLifecycle::new_with_runtime(
        ledger,
        FakePrivateClaurstBridge::default(),
        StoppingRuntime(std::sync::Arc::clone(&stopping)),
        "gentd-1".into(),
        HostEpoch(1),
    );
    lifecycle.activate_recovery().await.unwrap();
    let mut reap = std::pin::pin!(lifecycle.reap_idle(std::time::Instant::now()));
    let polled = std::future::Future::poll(
        reap.as_mut(),
        &mut std::task::Context::from_waker(std::task::Waker::noop()),
    );
    assert!(polled.is_ready());
    stopping.add_permits(1);
}
