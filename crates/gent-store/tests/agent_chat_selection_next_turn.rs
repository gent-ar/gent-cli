use gent_ports::{
    AgentChatConversationConfigLedger, AgentChatPromptDispatchLedger, AgentChatPromptLedger,
    AgentChatQueuedPromptLedger, AgentChatReadLedger, AgentChatRunContextReader,
    AgentChatSelectionLedger, AgentChatWorkspaceLedger, Ledger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationConfigRecord, AgentChatConversationCreate, AgentChatConversationId,
    AgentChatEffort, AgentChatMode, AgentChatPromptCreate, AgentChatPromptDisposition,
    AgentChatPromptSaved, AgentChatProvider, AgentChatRequestId, AgentChatRunId,
    AgentChatSelection, AgentChatSelectionSwitch, Containment, ContextPolicy, DurableTurnPhase,
    HostEpoch, ReceiptId, WorkspaceRecord,
};

const EPOCH: HostEpoch = HostEpoch(1);
const DAEMON: &str = "daemon-a";

fn selection(
    provider: AgentChatProvider,
    model: &str,
    effort: AgentChatEffort,
    mode: AgentChatMode,
) -> AgentChatSelection {
    AgentChatSelection {
        provider,
        model: model.into(),
        effort,
        mode,
    }
}

fn haiku() -> AgentChatSelection {
    selection(
        AgentChatProvider::Claude,
        "haiku",
        AgentChatEffort::Low,
        AgentChatMode::Ask,
    )
}

fn conversation() -> (SqliteLedger, AgentChatConversationId) {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-1".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create-receipt".into()),
                idempotency_key: "create-key".into(),
                host_epoch: EPOCH,
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run-1".into()),
                selection: haiku(),
            },
            &WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    (ledger, conversation_id)
}

fn save(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
    request: &str,
    disposition: AgentChatPromptDisposition,
) -> AgentChatPromptSaved {
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(request.into()),
            receipt_id: ReceiptId(format!("receipt-{request}")),
            host_epoch: EPOCH,
            conversation_id: conversation_id.clone(),
            disposition,
            text: request.into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap()
}

fn release(ledger: &SqliteLedger, saved: &AgentChatPromptSaved, run_id: &AgentChatRunId) {
    ledger
        .release_agent_chat_prompt_after_readiness(&saved.message.message_id, run_id, EPOCH)
        .unwrap();
}

fn claim(ledger: &SqliteLedger, provider: AgentChatProvider) -> Option<AgentChatPromptSaved> {
    ledger
        .claim_agent_chat_prompt_dispatch(DAEMON, EPOCH, provider)
        .unwrap()
}

fn start(ledger: &SqliteLedger, saved: &AgentChatPromptSaved) {
    ledger
        .begin_agent_chat_prompt_launch(&saved.message.message_id, DAEMON, EPOCH)
        .unwrap();
    ledger
        .confirm_agent_chat_prompt_started(&saved.message.message_id, DAEMON, EPOCH)
        .unwrap();
}

fn finish(ledger: &SqliteLedger, saved: &AgentChatPromptSaved) {
    ledger
        .settle_agent_chat_prompt_terminal(
            &saved.message.message_id,
            DAEMON,
            EPOCH,
            DurableTurnPhase::Completed,
        )
        .unwrap();
}

fn switch(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
    next: &AgentChatSelection,
    context_policy: ContextPolicy,
) -> AgentChatRunId {
    let run_id = AgentChatRunId("run-2".into());
    ledger
        .switch_agent_chat_selection(&AgentChatSelectionSwitch {
            receipt_id: ReceiptId("switch-receipt".into()),
            idempotency_key: "switch-key".into(),
            host_epoch: EPOCH,
            conversation_id: conversation_id.clone(),
            parent_run_id: AgentChatRunId("run-1".into()),
            run_id: run_id.clone(),
            selection: next.clone(),
            context_policy,
        })
        .unwrap();
    run_id
}

fn running_turn(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
) -> AgentChatPromptSaved {
    let running = save(
        ledger,
        conversation_id,
        "running",
        AgentChatPromptDisposition::Send,
    );
    release(ledger, &running, &running.run_id);
    let claimed = claim(ledger, AgentChatProvider::Claude).unwrap();
    assert_eq!(claimed.message, running.message);
    start(ledger, &claimed);
    claimed
}

fn applies_next_turn(ledger: &SqliteLedger, conversation_id: &AgentChatConversationId) -> bool {
    ledger
        .read_agent_chat_summary(&conversation_id.0)
        .unwrap()
        .selection_applies_next_turn
}

#[test]
fn every_selection_change_mid_turn_is_accepted_and_starts_with_the_next_turn() {
    let changes = [
        selection(
            AgentChatProvider::Codex,
            "gpt-5.6-luna",
            AgentChatEffort::Low,
            AgentChatMode::Ask,
        ),
        selection(
            AgentChatProvider::Claude,
            "claude-haiku-4-5",
            AgentChatEffort::Low,
            AgentChatMode::Ask,
        ),
        selection(
            AgentChatProvider::Claude,
            "haiku",
            AgentChatEffort::Medium,
            AgentChatMode::Ask,
        ),
        selection(
            AgentChatProvider::Claude,
            "haiku",
            AgentChatEffort::Low,
            AgentChatMode::Plan,
        ),
    ];
    for next in changes {
        let (ledger, conversation_id) = conversation();
        let running = running_turn(&ledger, &conversation_id);
        let queued_before = save(
            &ledger,
            &conversation_id,
            "queued-before",
            AgentChatPromptDisposition::Queue,
        );
        release(&ledger, &queued_before, &queued_before.run_id);
        let child = switch(&ledger, &conversation_id, &next, ContextPolicy::Preserve);
        let summary = ledger.read_agent_chat_summary(&conversation_id.0).unwrap();
        assert_eq!(summary.selection, next);
        assert!(summary.selection_applies_next_turn, "{next:?}");
        let queued_after = save(
            &ledger,
            &conversation_id,
            "queued-after",
            AgentChatPromptDisposition::Queue,
        );
        assert_eq!(queued_after.run_id, child);
        release(&ledger, &queued_after, &child);
        assert!(claim(&ledger, next.provider).is_none(), "{next:?}");
        assert!(
            claim(&ledger, AgentChatProvider::Claude).is_none(),
            "{next:?}"
        );

        finish(&ledger, &running);
        assert!(!applies_next_turn(&ledger, &conversation_id));
        let first = claim(&ledger, next.provider).unwrap();
        assert_eq!(first.message.message_id, queued_before.message.message_id);
        assert_eq!(first.run_id, child);
        start(&ledger, &first);
        finish(&ledger, &first);
        let second = claim(&ledger, next.provider).unwrap();
        assert_eq!(second.message.message_id, queued_after.message.message_id);
        assert_eq!(second.run_id, child);
        assert_eq!(
            ledger
                .read_agent_chat_detail(&conversation_id.0)
                .unwrap()
                .runs
                .last()
                .map(|run| (&run.run_id, &run.selection)),
            Some((&child.0, &next))
        );
    }
}

#[test]
fn a_clear_issued_mid_turn_freezes_no_history_for_the_next_turn() {
    let (ledger, conversation_id) = conversation();
    let running = running_turn(&ledger, &conversation_id);
    let child = switch(&ledger, &conversation_id, &haiku(), ContextPolicy::Clear);
    let context = ledger
        .read_agent_chat_run_context(&conversation_id, &child)
        .unwrap();
    assert_eq!(
        (context.context_policy, context.context_through_ordinal),
        (ContextPolicy::Clear, 0)
    );
    finish(&ledger, &running);
    assert!(!applies_next_turn(&ledger, &conversation_id));
}

#[test]
fn containment_saved_mid_turn_is_accepted_for_the_next_launch() {
    let (ledger, conversation_id) = conversation();
    let _running = running_turn(&ledger, &conversation_id);
    ledger
        .save_conversation_config(&AgentChatConversationConfigRecord {
            conversation_id: conversation_id.clone(),
            revision: 1,
            system_prompt: None,
            append_system_prompt: false,
            max_turns: None,
            thinking_enabled: None,
            disallowed_tools: vec![],
            containment: Containment::Sandboxed,
        })
        .unwrap();
    assert_eq!(
        ledger
            .current_conversation_config(&conversation_id.0)
            .unwrap()
            .unwrap()
            .containment,
        Containment::Sandboxed
    );
}

fn luna() -> AgentChatSelection {
    selection(
        AgentChatProvider::Codex,
        "gpt-5.6-luna",
        AgentChatEffort::Low,
        AgentChatMode::Ask,
    )
}

fn queue(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
    request: &str,
) -> AgentChatPromptSaved {
    let queued = save(
        ledger,
        conversation_id,
        request,
        AgentChatPromptDisposition::Queue,
    );
    release(ledger, &queued, &queued.run_id);
    queued
}

fn send_now(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
    queued: &AgentChatPromptSaved,
) -> AgentChatRunId {
    ledger
        .steer_queued_agent_chat_prompt(
            &ReceiptId(format!("steer-{}", queued.message.message_id)),
            EPOCH,
            conversation_id,
            &queued.message.message_id,
        )
        .unwrap()
        .1
}

fn claim_steer(ledger: &SqliteLedger, running: &AgentChatPromptSaved) -> Option<String> {
    ledger
        .claim_steered_agent_chat_prompt_for_turn(
            DAEMON,
            EPOCH,
            &running.run_id,
            &running.message.turn_id,
        )
        .unwrap()
        .map(|(prompt, _)| {
            assert_eq!(prompt.run_id, running.run_id);
            prompt.message.message_id
        })
}

#[test]
fn send_now_after_a_switch_steers_into_the_running_turn_in_queue_order_exactly_once() {
    let (ledger, conversation_id) = conversation();
    let running = running_turn(&ledger, &conversation_id);
    let carried = queue(&ledger, &conversation_id, "carried");
    let child = switch(&ledger, &conversation_id, &luna(), ContextPolicy::Preserve);
    let later = queue(&ledger, &conversation_id, "later");
    let untouched = queue(&ledger, &conversation_id, "untouched");
    assert_eq!(later.run_id, child);

    assert_eq!(send_now(&ledger, &conversation_id, &later), running.run_id);
    assert_eq!(claim_steer(&ledger, &running), None);
    assert_eq!(
        send_now(&ledger, &conversation_id, &carried),
        running.run_id
    );
    for expected in [&carried, &later] {
        let message_id = claim_steer(&ledger, &running).unwrap();
        assert_eq!(message_id, expected.message.message_id);
        ledger
            .deliver_steered_agent_chat_prompt(&message_id, DAEMON, EPOCH, &running.message.turn_id)
            .unwrap();
    }
    assert_eq!(claim_steer(&ledger, &running), None);
    assert!(claim(&ledger, AgentChatProvider::Codex).is_none());

    finish(&ledger, &running);
    let next = claim(&ledger, AgentChatProvider::Codex).unwrap();
    assert_eq!(next.message.message_id, untouched.message.message_id);
    assert_eq!(next.run_id, child);
    assert!(claim(&ledger, AgentChatProvider::Codex).is_none());
    assert!(claim(&ledger, AgentChatProvider::Claude).is_none());
}

#[test]
fn targeted_send_now_after_a_switch_steers_into_the_named_live_turn_only() {
    let (ledger, conversation_id) = conversation();
    let running = running_turn(&ledger, &conversation_id);
    let child = switch(&ledger, &conversation_id, &luna(), ContextPolicy::Preserve);
    let queued = queue(&ledger, &conversation_id, "queued");
    assert_eq!(queued.run_id, child);
    let target = |receipt: &str, run: &AgentChatRunId, turn: &str| {
        ledger.steer_queued_agent_chat_prompt_for_turn(
            &ReceiptId(receipt.into()),
            EPOCH,
            &conversation_id,
            &queued.message.message_id,
            run,
            turn,
        )
    };
    assert!(target("stale-turn", &running.run_id, "turn-stale").is_err());
    assert!(target("child-run", &child, &running.message.turn_id).is_err());
    let (_, run) = target("live-turn", &running.run_id, &running.message.turn_id).unwrap();
    assert_eq!(run, running.run_id);

    let message_id = claim_steer(&ledger, &running).unwrap();
    assert_eq!(message_id, queued.message.message_id);
    ledger
        .deliver_steered_agent_chat_prompt(&message_id, DAEMON, EPOCH, &running.message.turn_id)
        .unwrap();
    assert_eq!(claim_steer(&ledger, &running), None);
    finish(&ledger, &running);
    assert!(claim(&ledger, AgentChatProvider::Codex).is_none());
    assert!(claim(&ledger, AgentChatProvider::Claude).is_none());
}

#[test]
fn a_new_steer_after_a_switch_is_delivered_into_the_named_live_turn_only() {
    let (ledger, conversation_id) = conversation();
    let running = running_turn(&ledger, &conversation_id);
    let child = switch(&ledger, &conversation_id, &luna(), ContextPolicy::Preserve);
    let steer = |request: &str, run: &AgentChatRunId, turn: &str| {
        ledger.save_agent_chat_prompt_for_steer(
            &AgentChatPromptCreate {
                request_id: AgentChatRequestId(request.into()),
                receipt_id: ReceiptId(format!("receipt-{request}")),
                host_epoch: EPOCH,
                conversation_id: conversation_id.clone(),
                disposition: AgentChatPromptDisposition::Queue,
                text: request.into(),
                attachment_ids: vec![],
                tool_source_ids: vec![],
            },
            run,
            turn,
        )
    };
    assert!(steer("stale-turn", &running.run_id, "turn-stale").is_err());
    assert!(steer("child-run", &child, &running.message.turn_id).is_err());
    let saved = steer("live-turn", &running.run_id, &running.message.turn_id).unwrap();
    release(&ledger, &saved, &saved.run_id);

    let message_id = claim_steer(&ledger, &running).unwrap();
    assert_eq!(message_id, saved.message.message_id);
    ledger
        .deliver_steered_agent_chat_prompt(&message_id, DAEMON, EPOCH, &running.message.turn_id)
        .unwrap();
    assert_eq!(claim_steer(&ledger, &running), None);
    finish(&ledger, &running);
    assert!(claim(&ledger, AgentChatProvider::Codex).is_none());
    assert!(claim(&ledger, AgentChatProvider::Claude).is_none());
}

#[test]
fn a_rejected_send_now_returns_the_prompt_to_the_new_selection() {
    let (ledger, conversation_id) = conversation();
    let running = running_turn(&ledger, &conversation_id);
    let child = switch(&ledger, &conversation_id, &luna(), ContextPolicy::Preserve);
    let queued = queue(&ledger, &conversation_id, "queued");
    send_now(&ledger, &conversation_id, &queued);
    let message_id = claim_steer(&ledger, &running).unwrap();
    ledger
        .release_agent_chat_prompt_unstarted_launch(&message_id, DAEMON, EPOCH)
        .unwrap();
    assert!(claim(&ledger, AgentChatProvider::Codex).is_none());

    finish(&ledger, &running);
    let next = claim(&ledger, AgentChatProvider::Codex).unwrap();
    assert_eq!(
        (next.message.message_id, next.run_id),
        (queued.message.message_id, child)
    );
}

fn claimed_parent_prompt_then_switch() -> (SqliteLedger, AgentChatPromptSaved, AgentChatRunId) {
    let (ledger, conversation_id) = conversation();
    let parent = save(
        &ledger,
        &conversation_id,
        "parent",
        AgentChatPromptDisposition::Send,
    );
    release(&ledger, &parent, &parent.run_id);
    let claimed = claim(&ledger, AgentChatProvider::Claude).unwrap();
    let child = switch(&ledger, &conversation_id, &luna(), ContextPolicy::Preserve);
    (ledger, claimed, child)
}

#[test]
fn a_prompt_claimed_when_a_switch_lands_launches_and_completes_on_its_own_run() {
    let (ledger, claimed, child) = claimed_parent_prompt_then_switch();
    let conversation_id = AgentChatConversationId(claimed.message.conversation_id.clone());
    let queued = queue(&ledger, &conversation_id, "queued");
    start(&ledger, &claimed);
    assert_eq!(claimed.run_id, AgentChatRunId("run-1".into()));
    assert!(claim(&ledger, AgentChatProvider::Codex).is_none());
    finish(&ledger, &claimed);
    let next = claim(&ledger, AgentChatProvider::Codex).unwrap();
    assert_eq!(
        (next.message.message_id, next.run_id),
        (queued.message.message_id, child)
    );
}

#[test]
fn a_parent_prompt_released_before_launch_after_a_switch_runs_on_the_new_selection() {
    let (ledger, claimed, child) = claimed_parent_prompt_then_switch();
    ledger
        .release_agent_chat_prompt_claim(&claimed.message.message_id, DAEMON, EPOCH)
        .unwrap();
    let next = claim(&ledger, AgentChatProvider::Codex).unwrap();
    assert_eq!(
        (next.message.message_id, next.run_id),
        (claimed.message.message_id, child)
    );
}

#[test]
fn a_parent_prompt_claimed_when_the_daemon_crashed_recovers_onto_the_new_selection() {
    let (ledger, claimed, child) = claimed_parent_prompt_then_switch();
    ledger.close_ingress(EPOCH).unwrap();
    ledger.fence_and_open(EPOCH).unwrap();
    let restarted = HostEpoch(2);
    ledger
        .recover_agent_chat_prompt_dispatches(restarted)
        .unwrap();
    let next = ledger
        .claim_agent_chat_prompt_dispatch("daemon-b", restarted, AgentChatProvider::Codex)
        .unwrap()
        .unwrap();
    assert_eq!(
        (next.message.message_id, next.run_id),
        (claimed.message.message_id, child)
    );
}
