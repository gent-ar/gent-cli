use gent_types::{
    AgentChatConversationId, AgentChatProvider, AgentChatRunId, DurableTurnPhase,
    NormalizedTranscriptKind,
};
use serde_json::json;

use super::fake_cli::FakeCodexDaemon;
use crate::public_driver_runtime::run_failure::RUN_FAILURE_NOTICE;
use gent_types::PROVIDER_SESSION_UNAVAILABLE_NOTICE;

fn seeded(
    daemon: &mut FakeCodexDaemon,
    key: &str,
    text: &str,
) -> (AgentChatConversationId, AgentChatRunId, String) {
    let (conversation, run) = daemon.conversation(key);
    let seed = daemon.prompt(&conversation, text);
    assert_eq!(daemon.settle(&seed), DurableTurnPhase::Completed);
    let thread = daemon.bound_session(&run);
    (conversation, run, thread)
}

fn assert_resumed_once(daemon: &FakeCodexDaemon, thread: &str) {
    let starts = daemon.requests("thread/start");
    let resumes = daemon.requests("thread/resume");
    assert_eq!(
        starts.len(),
        1,
        "a bound run must never start another thread"
    );
    assert_eq!(resumes.len(), 1);
    assert_eq!(resumes[0]["params"]["threadId"], thread);
    assert_eq!(resumes[0]["params"]["excludeTurns"], true);
    assert_ne!(resumes[0]["launch"], starts[0]["launch"]);
}

#[test]
fn mcp_change_resumes_the_bound_thread_with_its_long_history() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, run, thread) =
        seeded(&mut daemon, "mcp", "Remember CODE-HERON and write LONG");
    daemon.set_mcp_servers(&json!({"docs": {"command": "docs-v2"}}));

    let recall = daemon.prompt(&conversation, "Which code did I give you?");

    assert_eq!(daemon.settle(&recall), DurableTurnPhase::Completed);
    assert_eq!(daemon.reply(&recall), "recall: CODE-HERON");
    assert_eq!(daemon.bound_session(&run), thread);
    assert_resumed_once(&daemon, &thread);
    let resume = &daemon.requests("thread/resume")[0];
    assert_eq!(
        resume["params"]["config"]["mcp_servers"]["docs"]["command"],
        "docs-v2"
    );
}

#[test]
fn interrupt_then_next_prompt_resumes_the_bound_thread_even_after_an_mcp_change() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, run, thread) = seeded(&mut daemon, "interrupt", "Remember CODE-PELICAN");
    let waiting = daemon.prompt(&conversation, "WAIT for me");
    daemon.drive_until("streamed output", |daemon| {
        !daemon.reply(&waiting).is_empty()
    });

    daemon
        .router
        .interrupt_run(AgentChatProvider::Codex, &run.0)
        .unwrap();
    assert_eq!(daemon.settle(&waiting), DurableTurnPhase::Interrupted);
    daemon.set_mcp_servers(&json!({"docs": {"command": "docs-v3"}}));
    let recall = daemon.prompt(&conversation, "Which code did I give you?");

    assert_eq!(daemon.settle(&recall), DurableTurnPhase::Completed);
    assert_eq!(daemon.reply(&recall), "recall: CODE-PELICAN");
    assert_eq!(daemon.bound_session(&run), thread);
    assert_resumed_once(&daemon, &thread);
    let resume = &daemon.requests("thread/resume")[0];
    assert_eq!(
        resume["params"]["config"]["mcp_servers"]["docs"]["command"],
        "docs-v3"
    );
    let injected = daemon.requests("thread/inject_items");
    assert_eq!(injected.len(), 1);
    assert_eq!(
        injected[0]["params"]["items"][0]["content"][0]["text"],
        "working"
    );
}

#[test]
fn the_next_turn_remembers_the_partial_reply_an_interrupt_cut_off() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, run, thread) = seeded(&mut daemon, "partial", "hello");
    let waiting = daemon.prompt(&conversation, "WAIT and tell me a SECRET");
    daemon.drive_until("the partial reply", |daemon| {
        daemon.reply(&waiting).contains("CODE-UNSAID")
    });
    daemon
        .router
        .interrupt_run(AgentChatProvider::Codex, &run.0)
        .unwrap();
    assert_eq!(daemon.settle(&waiting), DurableTurnPhase::Interrupted);

    let recall = daemon.prompt(&conversation, "Which code did you mention?");
    assert_eq!(daemon.settle(&recall), DurableTurnPhase::Completed);
    assert_eq!(daemon.reply(&recall), "recall: CODE-UNSAID");
    let again = daemon.prompt(&conversation, "And now?");
    assert_eq!(daemon.settle(&again), DurableTurnPhase::Completed);
    daemon.restart();
    let after_restart = daemon.prompt(&conversation, "After a restart?");
    assert_eq!(daemon.settle(&after_restart), DurableTurnPhase::Completed);

    let injected = daemon.requests("thread/inject_items");
    assert_eq!(
        injected.len(),
        1,
        "a partial reply is restored exactly once"
    );
    assert_eq!(injected[0]["params"]["threadId"], thread);
    assert_eq!(
        injected[0]["params"]["items"][0],
        json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "working on CODE-UNSAID"}]})
    );
}

#[test]
fn daemon_restart_resumes_the_bound_thread() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, run, thread) = seeded(&mut daemon, "restart", "Remember CODE-FALCON");
    daemon.restart();

    let recall = daemon.prompt(&conversation, "Which code did I give you?");

    assert_eq!(daemon.settle(&recall), DurableTurnPhase::Completed);
    assert_eq!(daemon.reply(&recall), "recall: CODE-FALCON");
    assert_eq!(daemon.bound_session(&run), thread);
    assert_resumed_once(&daemon, &thread);
}

#[test]
fn a_missing_bound_thread_fails_the_turn_with_a_typed_session_unavailable_cause_while_gentd_keeps_serving()
 {
    let mut daemon = FakeCodexDaemon::start();
    let (lost, lost_run, thread) = seeded(&mut daemon, "lost", "Remember CODE-OTTER");
    let (healthy, _) = daemon.conversation("healthy");
    daemon.lose_thread(&thread);
    daemon.restart();

    let next = daemon.prompt(&lost, "Which code did I give you?");

    assert_eq!(daemon.settle(&next), DurableTurnPhase::Failed);
    assert_eq!(daemon.bound_session(&lost_run), thread);
    assert_eq!(daemon.requests("thread/start").len(), 1);
    assert_eq!(daemon.requests("thread/turns/list").len(), 1);
    assert!(
        daemon.requests("turn/start").iter().all(|request| {
            request["params"]["input"][0]["text"] != "Which code did I give you?"
        })
    );
    assert!(daemon.transcript(&next).iter().any(|event| {
        event.kind == NormalizedTranscriptKind::Notice
            && event.text == PROVIDER_SESSION_UNAVAILABLE_NOTICE
    }));
    let projection = daemon.projection(&lost);
    assert!(projection.iter().any(|event| {
        event.payload["activity"]["type"] == "terminal"
            && event.payload["activity"]["phase"] == "failed"
            && event.payload["activity"]["cause"] == "providerSessionUnavailable"
            && event.payload["turnId"] == next.message.turn_id.as_str()
    }));
    let serialized = serde_json::to_string(&projection).unwrap();
    assert!(!serialized.contains(&thread));
    assert!(!serialized.contains("no rollout found"));

    let hello = daemon.prompt(&healthy, "hello");
    assert_eq!(daemon.settle(&hello), DurableTurnPhase::Completed);
}

#[test]
fn a_resume_rejected_while_the_thread_history_exists_stays_a_genuine_failure() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, _, _) = seeded(&mut daemon, "rejected", "Remember CODE-WREN");
    daemon.reject_resumes();
    daemon.restart();

    let next = daemon.prompt(&conversation, "Which code did I give you?");

    assert_eq!(daemon.settle(&next), DurableTurnPhase::Failed);
    assert_eq!(daemon.requests("thread/turns/list").len(), 1);
    assert!(daemon.transcript(&next).iter().any(|event| {
        event.kind == NormalizedTranscriptKind::Notice && event.text == RUN_FAILURE_NOTICE
    }));
    let projection = daemon.projection(&conversation);
    assert!(projection.iter().any(|event| {
        event.payload["activity"]["type"] == "terminal"
            && event.payload["turnId"] == next.message.turn_id.as_str()
            && event.payload["activity"].get("cause").is_none()
    }));
    assert!(!projection.iter().any(|event| {
        event.payload["lifecycle"]["event"]["classification"] == "sessionUnavailable"
    }));
}

#[test]
fn compact_runs_a_native_codex_compaction_turn_on_the_bound_thread_and_keeps_its_history() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, run, thread) = seeded(&mut daemon, "compact", "Remember CODE-OSPREY");

    let compaction = daemon.prompt(&conversation, "/compact");
    assert_eq!(daemon.settle(&compaction), DurableTurnPhase::Completed);

    let compactions = daemon.requests("thread/compact/start");
    assert_eq!(compactions.len(), 1);
    assert_eq!(compactions[0]["params"]["threadId"], thread);
    assert_eq!(daemon.requests("turn/start").len(), 1);
    let notices: Vec<String> = daemon
        .transcript(&compaction)
        .into_iter()
        .filter(|event| event.kind == NormalizedTranscriptKind::Notice)
        .map(|event| event.text)
        .collect();
    assert_eq!(notices, [gent_types::PROVIDER_CONTEXT_COMPACTED_NOTICE]);

    let recall = daemon.prompt(&conversation, "Which code did I give you?");
    assert_eq!(daemon.settle(&recall), DurableTurnPhase::Completed);
    assert_eq!(daemon.reply(&recall), "recall: CODE-OSPREY");
    assert_eq!(daemon.bound_session(&run), thread);
}
