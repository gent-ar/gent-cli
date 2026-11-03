use super::{ConversationActivityFact, ConversationActivityScope};
use crate::HostEpoch;

#[test]
fn tool_output_preserves_identity_and_partial_state_on_the_wire() {
    let value = serde_json::json!({
        "type": "toolOutput", "conversationId": "conversation-1", "runId": "run-1",
        "turnId": "turn-1", "hostEpoch": 1, "cursor": 3,
        "toolUseId": "tool-1", "text": "first\n  second", "isPartial": true
    });
    let fact = serde_json::from_value::<ConversationActivityFact>(value.clone()).unwrap();
    assert_eq!(fact.scope().cursor, 3);
    assert!(matches!(&fact, ConversationActivityFact::ToolOutput {
        tool_use_id, text, is_partial: true, ..
    } if tool_use_id == "tool-1" && text == "first\n  second"));
    assert_eq!(serde_json::to_value(fact).unwrap(), value);
}

#[test]
fn fact_round_trips_with_shared_scope() {
    let fact = ConversationActivityFact::DecisionPending {
        scope: ConversationActivityScope {
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            host_epoch: HostEpoch(9),
            cursor: 12,
        },
        decision_id: "decision-1".into(),
    };

    let value = serde_json::to_value(&fact).expect("fact serializes");
    assert_eq!(value["type"], "decisionPending");
    assert_eq!(value["conversationId"], "conversation-1");
    assert_eq!(value["hostEpoch"], 9);
    assert_eq!(value["cursor"], 12);
    assert_eq!(value["decisionId"], "decision-1");
    assert!(value.get("decision_id").is_none());
    assert_eq!(
        serde_json::from_value::<ConversationActivityFact>(value).expect("fact deserializes"),
        fact
    );
}

#[test]
fn a_terminal_cause_is_additive_and_absent_for_ordinary_endings() {
    let scope = ConversationActivityScope {
        conversation_id: "conversation-1".into(),
        run_id: "run-1".into(),
        turn_id: "turn-1".into(),
        host_epoch: HostEpoch(1),
        cursor: 3,
    };
    let steered = ConversationActivityFact::Terminal {
        scope: scope.clone(),
        phase: crate::TurnPhase::Interrupted,
        cause: Some(crate::TurnTerminalCause::Steered),
    };
    let value = serde_json::to_value(&steered).unwrap();
    assert_eq!(
        (value["phase"].clone(), value["cause"].clone()),
        ("interrupted".into(), "steered".into())
    );
    let stopped = serde_json::json!({
        "type": "terminal", "conversationId": "conversation-1", "runId": "run-1",
        "turnId": "turn-1", "hostEpoch": 1, "cursor": 3, "phase": "interrupted"
    });
    let parsed = serde_json::from_value::<ConversationActivityFact>(stopped.clone()).unwrap();
    assert!(matches!(
        parsed,
        ConversationActivityFact::Terminal { cause: None, .. }
    ));
    assert_eq!(serde_json::to_value(parsed).unwrap(), stopped);
}

#[test]
fn held_prompt_fact_names_its_prompt_and_typed_reason_without_secrets() {
    let fact = ConversationActivityFact::PromptHeld {
        scope: ConversationActivityScope {
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            host_epoch: HostEpoch(9),
            cursor: 12,
        },
        message_id: "message-1".into(),
        receipt_id: "prompt-receipt".into(),
        reason: crate::PromptHoldReason::ModelDownload,
    };

    assert_eq!(
        serde_json::to_value(&fact).expect("fact serializes"),
        serde_json::json!({
            "type": "promptHeld",
            "conversationId": "conversation-1",
            "runId": "run-1",
            "turnId": "turn-1",
            "hostEpoch": 9,
            "cursor": 12,
            "messageId": "message-1",
            "receiptId": "prompt-receipt",
            "reason": "modelDownload"
        })
    );
    assert_eq!(
        serde_json::from_value::<ConversationActivityFact>(
            serde_json::to_value(&fact).expect("fact serializes")
        )
        .expect("fact deserializes"),
        fact
    );
}
