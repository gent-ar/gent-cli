use gent_ports::{ConversationActivityLedger, MAX_CONVERSATION_ACTIVITY_PAGE_FACTS};
use gent_runtime::{Coordinator, RunLifecycleStatusService};
use gent_types::{
    AgentChatConversationId, AgentChatProjectionEvent, AgentChatPromptDisposition::Send,
    AgentChatRunId, CapabilitySet, ConversationActivityFact, ConversationLiveStatus,
    DurableTurnPhase, NormalizedTranscriptKind,
};
use serde_json::json;

use super::fake_cli::FakeClaudeDaemon;

const PARENT_TOOL: &str = "toolu_01VUYEGeCHLycLv5neJzkvz6";
const CHILD: &str = "afd78f7cc9d9fd901";
const CONTINUATION: &str = "The agent has completed. There are **3 .txt files** in the workspace.";

fn activity(
    daemon: &FakeClaudeDaemon,
    conversation: &AgentChatConversationId,
    run: &AgentChatRunId,
) -> Vec<ConversationActivityFact> {
    let mut facts = Vec::new();
    let mut after = 0;
    loop {
        let page = daemon
            .ledger
            .read_conversation_activity_page(
                &conversation.0,
                &run.0,
                after,
                MAX_CONVERSATION_ACTIVITY_PAGE_FACTS,
            )
            .unwrap();
        facts.extend(page.facts);
        match page.next_after_cursor {
            Some(next) => after = next,
            None => return facts,
        }
    }
}

fn live(daemon: &FakeClaudeDaemon, run: &AgentChatRunId) -> ConversationLiveStatus {
    RunLifecycleStatusService::new(Coordinator::new(
        daemon.ledger.clone(),
        CapabilitySet::default(),
    ))
    .live_status(&run.0)
    .unwrap()
    .unwrap()
    .status
}

fn child_settled(projection: &[AgentChatProjectionEvent]) -> bool {
    projection.iter().any(|event| {
        event.payload["activity"]["type"] == "workPhase"
            && event.payload["activity"]["workId"] == CHILD
    })
}

#[test]
fn background_subagent_completion_settles_the_child_after_the_root_turn() {
    let mut daemon = FakeClaudeDaemon::start();
    let (conversation, run) = daemon.conversation("background-subagent");
    let prompt = daemon.prompt(&conversation, "SUBAGENT count the txt files", Send);
    daemon.drive_until("root turn terminal", |daemon| {
        daemon.phase(&prompt).is_terminal()
    });
    assert_eq!(daemon.phase(&prompt), DurableTurnPhase::Completed);
    assert!(!child_settled(&daemon.projection(&conversation)));
    assert!(live(&daemon, &run).is_waiting_for_subagents());

    daemon.drive_until("continuation after the notification", |daemon| {
        daemon
            .transcript(&prompt)
            .iter()
            .any(|event| !event.is_partial && event.text == CONTINUATION)
    });

    assert!(!live(&daemon, &run).is_waiting_for_subagents());
    let facts = activity(&daemon, &conversation, &run);
    let settled: Vec<_> = facts
        .iter()
        .filter(|fact| matches!(fact, ConversationActivityFact::WorkPhase { work_id, .. } if work_id == CHILD))
        .map(|fact| serde_json::to_value(fact).unwrap())
        .filter(|fact| fact["phase"] != "running")
        .collect();
    let scope = facts
        .iter()
        .find(|fact| matches!(fact, ConversationActivityFact::SubagentStarted { .. }))
        .map(ConversationActivityFact::scope)
        .unwrap();
    assert_eq!(settled.len(), 1);
    assert_eq!(
        settled[0],
        json!({
            "type": "workPhase",
            "conversationId": conversation.0,
            "runId": run.0,
            "turnId": prompt.message.turn_id,
            "hostEpoch": daemon.epoch.0,
            "cursor": settled[0]["cursor"],
            "workId": CHILD,
            "kind": "subagent",
            "phase": "done",
        })
    );
    assert!(facts.iter().any(|fact| matches!(
        fact,
        ConversationActivityFact::SubagentStarted { child_id, parent_tool_use_id, .. }
            if child_id == CHILD && parent_tool_use_id == PARENT_TOOL
    )));
    let root_terminal = facts
        .iter()
        .find(|fact| matches!(fact, ConversationActivityFact::Terminal { .. }))
        .map(|fact| fact.scope().cursor)
        .unwrap();
    assert!(scope.cursor < root_terminal);
    assert!(root_terminal < settled[0]["cursor"].as_u64().unwrap());
    let projection = serde_json::to_string(&daemon.projection(&conversation)).unwrap();
    assert!(!projection.contains("unsupportedClaudeFrame"));
    assert!(!projection.contains("BackgroundTask"));
}

#[test]
fn background_subagent_tools_and_report_are_attributed_to_the_child() {
    let mut daemon = FakeClaudeDaemon::start();
    let (conversation, run) = daemon.conversation("child-attribution");
    let prompt = daemon.prompt(&conversation, "SUBAGENT count the txt files", Send);
    daemon.drive_until("continuation after the notification", |daemon| {
        daemon
            .transcript(&prompt)
            .iter()
            .any(|event| !event.is_partial && event.text == CONTINUATION)
    });

    let child_tools: Vec<_> = activity(&daemon, &conversation, &run)
        .into_iter()
        .filter_map(|fact| match fact {
            ConversationActivityFact::ToolActivity { activity, .. }
                if activity.tool_use_id != PARENT_TOOL =>
            {
                Some((activity.tool_name, activity.parent_tool_use_id))
            }
            _ => None,
        })
        .collect();
    assert_eq!(child_tools.len(), 4);
    assert!(child_tools.iter().all(|(name, parent)| {
        ["ToolSearch", "Bash"].contains(&name.as_str()) && parent.as_deref() == Some(PARENT_TOOL)
    }));
    let transcript = daemon.transcript(&prompt);
    let answers: Vec<_> = transcript
        .iter()
        .filter(|event| {
            !event.is_partial && event.kind == NormalizedTranscriptKind::AssistantMessage
        })
        .map(|event| event.text.as_str())
        .collect();
    assert_eq!(answers.len(), 2);
    assert!(answers[0].starts_with("I've launched a background agent"));
    assert_eq!(answers[1], CONTINUATION);
    assert!(transcript.iter().any(|event| {
        event.kind == NormalizedTranscriptKind::ToolActivity && event.text == "3"
    }));
    assert!(daemon.projection(&conversation).iter().any(|event| {
        event.payload["lifecycle"]["event"]["type"] == "toolOutputDelta"
            && event.payload["lifecycle"]["event"]["tool_use_id"] == PARENT_TOOL
            && event.payload["lifecycle"]["event"]["text"] == "3"
    }));
}
