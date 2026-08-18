use gent_ports::{AgentChatRunContextReader, ConversationContentReader, ConversationLedger};
use gent_runtime::{
    AgentChatConversationAuthority, AgentChatConversationRequest, AgentChatConversationResult,
    AgentChatConversationService, AgentChatPromptAuthority, AgentChatPromptRequest,
    AgentChatPromptResult, AgentChatPromptService, AgentChatReadService,
    AgentChatRunContextService, AgentChatSelectionSwitchAuthority, AgentChatSelectionSwitchRequest,
    AgentChatSelectionSwitchResult, AgentChatSelectionSwitchService,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatPromptDisposition,
    AgentChatProvider, AgentChatRequestId, AgentChatRunId, AgentChatSelection, ContextPolicy,
    HostEpoch, ReceiptId,
};

fn selection(provider: AgentChatProvider, model: &str) -> AgentChatSelection {
    AgentChatSelection {
        provider,
        model: model.into(),
        effort: AgentChatEffort::Low,
        mode: AgentChatMode::Ask,
    }
}

fn conversation(ledger: SqliteLedger) -> (AgentChatConversationId, AgentChatRunId) {
    let result =
        AgentChatConversationService::new(ledger, AgentChatConversationAuthority::Approved)
            .create(&AgentChatConversationRequest {
                request_id: AgentChatRequestId("conversation".into()),
                receipt_id: ReceiptId("conversation-receipt".into()),
                host_epoch: HostEpoch(1),
                selection: selection(AgentChatProvider::Claude, "haiku"),
            })
            .unwrap();
    let AgentChatConversationResult::Created(created) = result else {
        panic!("approved authority must create a conversation");
    };
    (created.conversation_id, created.run_id)
}

fn save(
    ledger: SqliteLedger,
    conversation_id: AgentChatConversationId,
    request_id: &str,
) -> AgentChatRunId {
    let result = AgentChatPromptService::new(ledger, AgentChatPromptAuthority::Approved)
        .submit(&AgentChatPromptRequest {
            request_id: AgentChatRequestId(request_id.into()),
            receipt_id: ReceiptId(format!("{request_id}-receipt")),
            host_epoch: HostEpoch(1),
            conversation_id,
            disposition: AgentChatPromptDisposition::Send,
            text: format!("prompt {request_id}"),
        })
        .unwrap();
    let AgentChatPromptResult::Saved(saved) = result else {
        panic!("approved authority must save a prompt");
    };
    saved.run_id
}

fn request(
    conversation_id: AgentChatConversationId,
    parent_run_id: AgentChatRunId,
) -> AgentChatSelectionSwitchRequest {
    AgentChatSelectionSwitchRequest {
        request_id: AgentChatRequestId("switch".into()),
        receipt_id: ReceiptId("switch-receipt".into()),
        host_epoch: HostEpoch(1),
        conversation_id,
        parent_run_id,
        selection: AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::High,
            mode: AgentChatMode::Plan,
        },
        context_policy: ContextPolicy::Preserve,
    }
}

#[test]
fn switch_creates_a_retry_stable_child_and_freezes_prior_history() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let (conversation_id, root_run_id) = conversation(ledger.clone());
    assert_eq!(
        save(ledger.clone(), conversation_id.clone(), "before"),
        root_run_id
    );
    let service = AgentChatSelectionSwitchService::new(
        ledger.clone(),
        AgentChatSelectionSwitchAuthority::Approved,
    );
    let request = request(conversation_id.clone(), root_run_id.clone());
    let first = service.switch(&request).unwrap();
    let second = service.switch(&request).unwrap();
    assert_eq!(first, second);
    let AgentChatSelectionSwitchResult::Switched(switched) = first else {
        panic!("approved authority must create a selected child run");
    };
    assert_eq!(switched.context_through_ordinal, 1);
    assert_eq!(switched.context_policy, ContextPolicy::Preserve);
    assert_eq!(switched.parent_run_id, root_run_id);
    assert_eq!(switched.selection.provider, AgentChatProvider::Codex);
    assert_eq!(switched.selection.mode, AgentChatMode::Plan);
    assert_eq!(switched.selection.effort, AgentChatEffort::High);
    assert_eq!(
        save(ledger.clone(), conversation_id.clone(), "after"),
        switched.run_id
    );
    let content = ledger
        .read_conversation_content(&conversation_id.0, Some(2), 10)
        .unwrap();
    assert_eq!(content.entries.len(), 1);
    assert_eq!(content.entries[0].text, "prompt before");
    let runs = ledger.list_conversation_runs(&conversation_id.0).unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].run_id, root_run_id.0);
    assert_eq!(
        runs[1].parent_run_id.as_deref(),
        Some(root_run_id.0.as_str())
    );
}

#[test]
fn clear_switch_creates_an_empty_context_child() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let (conversation_id, root_run_id) = conversation(ledger.clone());
    save(ledger.clone(), conversation_id.clone(), "before");
    let service =
        AgentChatSelectionSwitchService::new(ledger, AgentChatSelectionSwitchAuthority::Approved);
    let mut request = request(conversation_id, root_run_id);
    request.context_policy = ContextPolicy::Clear;
    request.request_id = AgentChatRequestId("clear".into());
    request.receipt_id = ReceiptId("clear-receipt".into());
    let AgentChatSelectionSwitchResult::Switched(switched) = service.switch(&request).unwrap()
    else {
        panic!("approved authority must create a selected child run");
    };
    assert_eq!(switched.context_policy, ContextPolicy::Clear);
    assert_eq!(switched.context_through_ordinal, 0);
    assert_eq!(
        service.switch(&request).unwrap(),
        AgentChatSelectionSwitchResult::Switched(switched)
    );
    let mut changed_retry = request;
    changed_retry.context_policy = ContextPolicy::Preserve;
    assert!(service.switch(&changed_retry).is_err());
}

#[test]
fn stale_parent_and_observer_switches_cannot_create_a_child() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let (conversation_id, root_run_id) = conversation(ledger.clone());
    let request = request(conversation_id.clone(), root_run_id.clone());
    let observer = AgentChatSelectionSwitchService::new(
        ledger.clone(),
        AgentChatSelectionSwitchAuthority::Observer,
    );
    assert_eq!(
        observer.switch(&request).unwrap(),
        AgentChatSelectionSwitchResult::DeniedObserver
    );
    let approved = AgentChatSelectionSwitchService::new(
        ledger.clone(),
        AgentChatSelectionSwitchAuthority::Approved,
    );
    approved.switch(&request).unwrap();
    let mut stale = request;
    stale.request_id = AgentChatRequestId("stale".into());
    stale.receipt_id = ReceiptId("stale-receipt".into());
    assert!(approved.switch(&stale).is_err());
    assert_eq!(
        ledger
            .list_conversation_runs(&conversation_id.0)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn selection_changes_create_fresh_children_for_claude_preserve_and_claurst_clear() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let (conversation_id, root_run_id) = conversation(ledger.clone());
    save(ledger.clone(), conversation_id.clone(), "before");
    let service = AgentChatSelectionSwitchService::new(
        ledger.clone(),
        AgentChatSelectionSwitchAuthority::Approved,
    );
    let mut preserve = request(conversation_id.clone(), root_run_id);
    preserve.request_id = AgentChatRequestId("claude-sonnet".into());
    preserve.receipt_id = ReceiptId("claude-sonnet-receipt".into());
    preserve.selection = AgentChatSelection {
        provider: AgentChatProvider::Claude,
        model: "sonnet".into(),
        effort: AgentChatEffort::High,
        mode: AgentChatMode::Agent,
    };
    let AgentChatSelectionSwitchResult::Switched(claude) = service.switch(&preserve).unwrap()
    else {
        panic!("switch must create a child")
    };
    assert_eq!(
        AgentChatReadService::new(ledger.clone())
            .run_selection(&conversation_id.0, &claude.run_id.0)
            .unwrap(),
        preserve.selection
    );
    let context = AgentChatRunContextService::new(ledger.clone())
        .resolve(&conversation_id, &claude.run_id)
        .unwrap();
    assert!(context.requires_fresh_provider_session());
    assert_eq!(
        (context.context_policy, context.context_through_ordinal),
        (ContextPolicy::Preserve, 1)
    );

    let mut clear = request(conversation_id.clone(), claude.run_id);
    clear.request_id = AgentChatRequestId("claurst-clear".into());
    clear.receipt_id = ReceiptId("claurst-clear-receipt".into());
    clear.context_policy = ContextPolicy::Clear;
    clear.selection = AgentChatSelection {
        provider: AgentChatProvider::Claurst,
        model: "private-model".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Plan,
    };
    let AgentChatSelectionSwitchResult::Switched(claurst) = service.switch(&clear).unwrap() else {
        panic!("switch must create a child")
    };
    assert_eq!(
        AgentChatReadService::new(ledger.clone())
            .run_selection(&conversation_id.0, &claurst.run_id.0)
            .unwrap(),
        clear.selection
    );
    assert_eq!(
        ledger
            .read_agent_chat_run_context(&conversation_id, &claurst.run_id)
            .unwrap()
            .context_through_ordinal,
        0
    );
}
