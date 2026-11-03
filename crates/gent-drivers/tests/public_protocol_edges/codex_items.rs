use super::*;

#[test]
fn codex_current_item_states_and_plan_compaction_remain_content_safe() {
    for kind in [
        "dynamicToolCall",
        "collabAgentToolCall",
        "webSearch",
        "imageGeneration",
    ] {
        let started = facts(
            PublicProvider::Codex,
            &json!({"method":"item/started","params":{"item":{"type":kind,"id":"tool"}}}),
        );
        assert!(started.iter().any(|fact| matches!(fact, PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) if activity.tool_name == kind && activity.phase == ToolPhase::Started)));
    }
    let failed = facts(
        PublicProvider::Codex,
        &json!({"method":"item/completed","params":{"item":{"type":"commandExecution","id":"tool","status":"failed","command":"secret"}}}),
    );
    assert!(failed.iter().any(|fact| matches!(fact, PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) if activity.phase == ToolPhase::Failed)));
    let frame = json!({"method":"thread/compacted","params":{"threadId":"secret-thread"}});
    let normalized = facts(PublicProvider::Codex, &frame);
    assert_eq!(
        normalized,
        vec![PublicWireFact::Compaction(
            gent_drivers::public_protocol::PublicCompactionObservation::Completed
        )]
    );
    assert!(!format!("{normalized:?}").contains("secret"));
    for method in [
        "item/plan/delta",
        "item/reasoning/textDelta",
        "item/reasoning/summaryTextDelta",
    ] {
        assert_eq!(
            facts(
                PublicProvider::Codex,
                &json!({"method":method,"params":{"delta":"working through it"}}),
            ),
            vec![PublicWireFact::Event(NormalizedProviderEvent::Thinking {
                text: "working through it".into(),
                is_partial: true,
            })],
        );
    }
    assert!(
        facts(
            PublicProvider::Codex,
            &json!({"method":"item/reasoning/summaryPartAdded","params":{}}),
        )
        .is_empty()
    );
    for method in [
        "item/commandExecution/outputDelta",
        "item/commandExecution/terminalInteraction",
        "item/fileChange/outputDelta",
    ] {
        let normalized = facts(
            PublicProvider::Codex,
            &json!({"method":method,"params":{"itemId":"tool","delta":"private"}}),
        );
        assert_eq!(
            normalized,
            vec![PublicWireFact::Event(
                NormalizedProviderEvent::ToolOutputDelta {
                    tool_use_id: "tool".into(),
                    text: "private".into(),
                    is_partial: true,
                }
            )],
            "{method} is a live tool-output frame"
        );
    }
    assert_eq!(
        facts(
            PublicProvider::Codex,
            &json!({"method":"item/fileChange/patchUpdated","params":{"itemId":"tool","changes":[{"diff":"private"}]}}),
        ),
        vec![PublicWireFact::Event(
            NormalizedProviderEvent::ToolOutputDelta {
                tool_use_id: "tool".into(),
                text: "private".into(),
                is_partial: true,
            }
        )]
    );
    assert!(
        facts(
            PublicProvider::Codex,
            &json!({"method":"item/tool/call","params":{"itemId":"tool","delta":"private"}}),
        )
        .is_empty()
    );
    assert_eq!(
        facts(
            PublicProvider::Codex,
            &json!({"method":"turn/plan/updated","params":{"plan":[{"step":"review durable state","status":"inProgress"}]}}),
        ),
        vec![PublicWireFact::Event(
            NormalizedProviderEvent::PlanProposed {
                text: "1. [inProgress] review durable state".into(),
            }
        )],
    );
    assert_eq!(
        facts(
            PublicProvider::Codex,
            &json!({"method":"item/started","params":{"item":{"type":"contextCompaction","secret":"ignored"}}}),
        ),
        vec![PublicWireFact::Compaction(
            gent_drivers::public_protocol::PublicCompactionObservation::Started
        )]
    );
    assert_eq!(
        facts(
            PublicProvider::Codex,
            &json!({"method":"item/completed","params":{"item":{"type":"contextCompaction","status":"failed","secret":"ignored"}}}),
        ),
        vec![PublicWireFact::Compaction(
            gent_drivers::public_protocol::PublicCompactionObservation::Failed {
                failure: AgentChatCompactionFailure::ProviderFailed,
            }
        )]
    );
}
