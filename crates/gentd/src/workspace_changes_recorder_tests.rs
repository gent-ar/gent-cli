use std::{fs, process::Command};

use gent_ports::{
    AgentChatProjectionLedger, AgentChatPromptDispatchLedger, AgentChatPromptLedger,
    AgentChatWorkspaceLedger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, DurableTurnPhase, HostEpoch, ReceiptId,
    WorkspaceFileChange, WorkspaceRecord,
};

use super::{capture_baseline, record_after};

fn git(directory: &std::path::Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .args([
                "-c",
                "user.name=Gent",
                "-c",
                "user.email=gent@example.invalid"
            ])
            .args(args)
            .current_dir(directory)
            .status()
            .unwrap()
            .success()
    );
}

fn conversation(
    ledger: &SqliteLedger,
    key: &str,
    workspace: &std::path::Path,
) -> AgentChatConversationId {
    let conversation_id = AgentChatConversationId(format!("conversation-{key}"));
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId(format!("create-{key}")),
                idempotency_key: format!("create-{key}"),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId(format!("run-{key}")),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "haiku".into(),
                    effort: AgentChatEffort::Low,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: format!("workspace-{key}"),
                canonical_path: workspace.display().to_string(),
            },
        )
        .unwrap();
    conversation_id
}

fn finished_turn(ledger: &SqliteLedger, conversation_id: &AgentChatConversationId, key: &str) {
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(format!("request-{key}")),
            receipt_id: ReceiptId(format!("receipt-{key}")),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: "write files".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    crate::readiness_test_support::release(ledger, &saved);
    ledger
        .claim_agent_chat_prompt_dispatch("daemon", HostEpoch(1), AgentChatProvider::Claude)
        .unwrap()
        .unwrap();
    let id = &saved.message.message_id;
    ledger
        .begin_agent_chat_prompt_launch(id, "daemon", HostEpoch(1))
        .unwrap();
    ledger
        .confirm_agent_chat_prompt_started(id, "daemon", HostEpoch(1))
        .unwrap();
    ledger
        .settle_agent_chat_prompt_terminal(id, "daemon", HostEpoch(1), DurableTurnPhase::Completed)
        .unwrap();
}

fn workspace_changes(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
) -> Vec<serde_json::Value> {
    ledger
        .agent_chat_projection_page(conversation_id, 0, 64)
        .unwrap()
        .events
        .into_iter()
        .map(|event| event.payload["activity"].clone())
        .filter(|activity| activity["type"] == "workspaceChanges")
        .collect()
}

#[test]
fn a_finished_turn_records_the_files_a_shell_wrote_since_the_first_prompt() {
    let directory = tempfile::tempdir().unwrap();
    let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
    let repository = tempfile::tempdir().unwrap();
    let workspace = repository.path().canonicalize().unwrap();
    git(&workspace, &["init", "--quiet"]);
    fs::write(workspace.join("existing.txt"), "kept\n").unwrap();
    git(&workspace, &["add", "--all"]);
    git(&workspace, &["commit", "--quiet", "-m", "base"]);
    fs::write(workspace.join("user-draft.txt"), "edited before the chat\n").unwrap();
    let conversation_id = conversation(&ledger, "git", &workspace);
    let plain = tempfile::tempdir().unwrap();
    let unversioned = conversation(&ledger, "plain", &plain.path().canonicalize().unwrap());
    let head = ledger.agent_chat_projection_head().unwrap();

    capture_baseline(&ledger, &conversation_id.0, HostEpoch(1)).unwrap();
    capture_baseline(&ledger, &unversioned.0, HostEpoch(1)).unwrap();
    fs::write(workspace.join("heredoc.txt"), "one\ntwo\n").unwrap();
    fs::write(workspace.join("existing.txt"), "kept\nmore\n").unwrap();
    finished_turn(&ledger, &conversation_id, "git");
    finished_turn(&ledger, &unversioned, "plain");

    let cursor = record_after(&ledger, head).unwrap();
    assert!(cursor > head);
    let recorded = workspace_changes(&ledger, &conversation_id);
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        recorded[0]["files"],
        serde_json::to_value(vec![
            WorkspaceFileChange {
                path: "existing.txt".into(),
                added: 1,
                removed: 0,
            },
            WorkspaceFileChange {
                path: "heredoc.txt".into(),
                added: 2,
                removed: 0,
            },
        ])
        .unwrap()
    );
    assert_eq!(recorded[0]["truncated"], false);
    assert!(workspace_changes(&ledger, &unversioned).is_empty());
    assert_eq!(record_after(&ledger, cursor).unwrap(), cursor);
}
