use gent_runtime::{
    AgentChatConversationAuthority, AgentChatConversationRequest, AgentChatConversationResult,
    AgentChatConversationService, AgentChatPromptAuthority, AgentChatPromptRequest,
    AgentChatPromptResult, AgentChatPromptService, AgentChatRunContextService,
    AgentChatSelectionSwitchAuthority, AgentChatSelectionSwitchRequest,
    AgentChatSelectionSwitchResult, AgentChatSelectionSwitchService,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatPromptDisposition, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, ContextPolicy, HostEpoch, ReceiptId,
};

#[test]
fn codex_claude_claurst_codex_preserves_frozen_neutral_history() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let (conversation, codex) = create(ledger.clone());
    save(ledger.clone(), &conversation, "codex-message");
    let switches = AgentChatSelectionSwitchService::new(
        ledger.clone(),
        AgentChatSelectionSwitchAuthority::Approved,
    );
    let claude = switch(
        &switches,
        &conversation,
        codex,
        "claude",
        AgentChatProvider::Claude,
        "sonnet",
        AgentChatMode::Plan,
    );
    save(ledger.clone(), &conversation, "claude-message");
    let claurst = switch(
        &switches,
        &conversation,
        claude,
        "claurst",
        AgentChatProvider::Claurst,
        "private-model",
        AgentChatMode::Agent,
    );
    save(ledger.clone(), &conversation, "claurst-message");
    let codex = switch(
        &switches,
        &conversation,
        claurst,
        "codex-return",
        AgentChatProvider::Codex,
        "gpt-5.6",
        AgentChatMode::Ask,
    );
    let context = AgentChatRunContextService::new(ledger)
        .resolve(&conversation, &codex)
        .unwrap();
    assert_eq!(
        (context.context_policy, context.context_through_ordinal),
        (ContextPolicy::Preserve, 3)
    );
    assert!(context.requires_fresh_provider_session());
}

fn create(ledger: SqliteLedger) -> (gent_types::AgentChatConversationId, AgentChatRunId) {
    let result =
        AgentChatConversationService::new(ledger, AgentChatConversationAuthority::Approved)
            .create(&AgentChatConversationRequest {
                request_id: AgentChatRequestId("create".into()),
                receipt_id: ReceiptId("create-receipt".into()),
                host_epoch: HostEpoch(1),
                selection: selection(AgentChatProvider::Codex, "gpt-5.6", AgentChatMode::Ask),
            })
            .unwrap();
    let AgentChatConversationResult::Created(created) = result else {
        panic!("conversation must be created")
    };
    (created.conversation_id, created.run_id)
}

fn save(ledger: SqliteLedger, conversation_id: &gent_types::AgentChatConversationId, id: &str) {
    let result = AgentChatPromptService::new(ledger, AgentChatPromptAuthority::Approved)
        .submit(&AgentChatPromptRequest {
            request_id: AgentChatRequestId(id.into()),
            receipt_id: ReceiptId(format!("{id}-receipt")),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: id.into(),
        })
        .unwrap();
    assert!(matches!(result, AgentChatPromptResult::Saved(_)));
}

fn switch(
    service: &AgentChatSelectionSwitchService<SqliteLedger>,
    conversation_id: &gent_types::AgentChatConversationId,
    parent_run_id: AgentChatRunId,
    id: &str,
    provider: AgentChatProvider,
    model: &str,
    mode: AgentChatMode,
) -> AgentChatRunId {
    let result = service
        .switch(&AgentChatSelectionSwitchRequest {
            request_id: AgentChatRequestId(id.into()),
            receipt_id: ReceiptId(format!("{id}-receipt")),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            parent_run_id,
            selection: selection(provider, model, mode),
            context_policy: ContextPolicy::Preserve,
        })
        .unwrap();
    let AgentChatSelectionSwitchResult::Switched(result) = result else {
        panic!("selection must switch")
    };
    result.run_id
}

fn selection(provider: AgentChatProvider, model: &str, mode: AgentChatMode) -> AgentChatSelection {
    AgentChatSelection {
        provider,
        model: model.into(),
        effort: AgentChatEffort::Medium,
        mode,
    }
}
