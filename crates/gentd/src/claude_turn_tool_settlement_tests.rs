use super::*;
use gent_types::{ConversationActivityFact, ToolActivity, ToolPhase};

fn tool(id: &str, name: &str, phase: ToolPhase, parent: Option<&str>) -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::ToolActivity {
            activity: ToolActivity {
                tool_use_id: id.into(),
                tool_name: name.into(),
                phase,
                input: None,
                output_digest: None,
                parent_tool_use_id: parent.map(Into::into),
            },
        },
    ))
}

fn tool_phases_then_terminal(scenario: &Scenario) -> Vec<(String, String)> {
    scenario
        .ledger
        .read_conversation_activity_page("conversation", "run", 0, 50)
        .unwrap()
        .facts
        .into_iter()
        .filter_map(|fact| match fact {
            ConversationActivityFact::ToolActivity { activity, .. } => Some((
                activity.tool_use_id,
                serde_json::to_value(activity.phase)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned(),
            )),
            ConversationActivityFact::Terminal { phase, .. } => Some((
                "terminal".into(),
                serde_json::to_value(phase)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned(),
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn an_interrupted_turn_settles_every_open_child_tool_before_its_terminal() {
    let mut scenario = joined_scenario();
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        tool("agent", "Agent", ToolPhase::Started, None),
        tool("child", "Bash", ToolPhase::WaitingPermission, Some("agent")),
        tool("child", "Bash", ToolPhase::Started, Some("agent")),
        tool("done", "Read", ToolPhase::Started, None),
        tool("done", "Read", ToolPhase::Completed, None),
    ]);
    scenario.host.tick().unwrap();
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        tool("agent", "Agent", ToolPhase::Failed, None),
        phase(TurnPhase::Interrupted),
    ]);
    scenario.host.tick().unwrap();
    let facts = tool_phases_then_terminal(&scenario);
    let tail = &facts[facts.len() - 3..];
    assert_eq!(
        tail,
        [
            ("agent".to_owned(), "failed".to_owned()),
            ("child".to_owned(), "failed".to_owned()),
            ("terminal".to_owned(), "interrupted".to_owned()),
        ]
    );
}

#[test]
fn a_completed_turn_leaves_background_child_tools_running() {
    let mut scenario = joined_scenario();
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        tool("agent", "Agent", ToolPhase::Started, None),
        tool("agent", "Agent", ToolPhase::Completed, None),
        tool("child", "Bash", ToolPhase::Started, Some("agent")),
        phase(TurnPhase::Ready),
    ]);
    scenario.host.tick().unwrap();
    let facts = tool_phases_then_terminal(&scenario);
    assert!(!facts.contains(&("child".to_owned(), "failed".to_owned())));
}
