use super::*;
use gent_ports::ConversationActivityLedger;

const CHILD: &str = "toolu_agent";

async fn child_asks_after_its_parent_turn_ended(
    mode: AgentChatMode,
    policy: PermissionMode,
) -> (SqliteLedger, FakePrivateClaurstBridge) {
    let (ledger, bridge, mut ingress) = prepare_permissions(mode, policy, vec![]).await;
    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![],
        permissions: vec![],
        checkpoint: Some(checkpoint(0)),
        session_binding: Some(binding()),
        terminal: Some(gent_ports::ClaurstTerminal::Completed),
    });
    let ended = ingress
        .drain(&binding().source_id, HostEpoch(1))
        .await
        .unwrap();
    assert!(ended.terminal);
    let mut child = permission(
        "edit-1",
        "Edit",
        PermissionCategory::Edit,
        serde_json::json!({"file_path": "/workspace-a/notes.txt"}),
    );
    child.child_id = Some(CHILD.into());
    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![],
        permissions: vec![child],
        checkpoint: Some(checkpoint(0)),
        session_binding: Some(binding()),
        terminal: None,
    });
    ingress
        .drain_background_commands(&binding().source_id, HostEpoch(1))
        .await
        .unwrap();
    (ledger, bridge)
}

#[tokio::test]
async fn a_childs_request_after_its_parent_turn_is_a_visible_decision_bound_to_the_child() {
    let (ledger, bridge) =
        child_asks_after_its_parent_turn_ended(AgentChatMode::Ask, PermissionMode::Bypass).await;

    assert!(bridge.permission_replies().is_empty());
    assert!(pending(&ledger));
    let request = ledger
        .pending_permission(
            &AgentChatConversationId("conversation-a".into()),
            &AgentChatRunId("run-a".into()),
        )
        .unwrap()
        .unwrap();
    assert_eq!(request.request.child_id.as_deref(), Some(CHILD));
}

#[tokio::test]
async fn a_childs_request_after_its_parent_turn_follows_the_conversation_policy() {
    let (ledger, bridge) = child_asks_after_its_parent_turn_ended(
        AgentChatMode::Agent,
        PermissionMode::AutoAcceptEdits,
    )
    .await;

    assert_eq!(
        bridge.permission_replies(),
        [("edit-1".to_owned(), ClaurstPermissionReply::AllowOnce)]
    );
    assert!(!pending(&ledger));
}

#[tokio::test]
async fn a_childs_tool_start_drained_with_its_request_stays_waiting_for_the_user() {
    let (ledger, bridge, mut ingress) =
        prepare_permissions(AgentChatMode::Ask, PermissionMode::AskEveryTime, vec![]).await;
    let mut child = permission(
        "edit-1",
        "Write",
        PermissionCategory::Edit,
        serde_json::json!({"file_path": "/workspace-a/child.txt", "content": "hi"}),
    );
    child.child_id = Some(CHILD.into());
    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![gent_ports::ClaurstNormalizedFact {
            source_id: gent_ports::ClaurstSourceId("source-a".into()),
            cursor: 1,
            value: gent_ports::ClaurstFactValue::Lifecycle(
                gent_types::NormalizedLifecycleSignal::ToolActivity {
                    activity: gent_types::ToolActivity {
                        tool_use_id: "tool-edit-1".into(),
                        tool_name: "Write".into(),
                        phase: gent_types::ToolPhase::Started,
                        output_digest: None,
                        input: None,
                        parent_tool_use_id: Some(CHILD.into()),
                    },
                },
            ),
        }],
        permissions: vec![child],
        checkpoint: Some(checkpoint(1)),
        session_binding: Some(binding()),
        terminal: None,
    });

    ingress
        .drain(&binding().source_id, HostEpoch(1))
        .await
        .unwrap();

    let phases: Vec<_> = ledger
        .read_conversation_activity_page("conversation-a", "run-a", 0, 64)
        .unwrap()
        .facts
        .into_iter()
        .filter_map(|fact| match fact {
            gent_types::ConversationActivityFact::ToolActivity { activity, .. }
                if activity.tool_use_id == "tool-edit-1" =>
            {
                Some(activity.phase)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        phases,
        [
            gent_types::ToolPhase::Started,
            gent_types::ToolPhase::WaitingPermission
        ]
    );
    assert!(pending(&ledger));
    assert!(bridge.permission_replies().is_empty());
}
