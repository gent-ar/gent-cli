use gent_ports::PendingPermissionLedger;
use gent_runtime::{TurnFollowRequest, TurnFollowService};
use gent_types::{
    AgentChatPromptDisposition::{Queue, Send},
    AgentChatPromptSaved, AgentChatProvider, DurableTurnPhase, NormalizedTranscriptKind,
};

use super::fake_cli::FakeClaudeDaemon;
use crate::public_driver_runtime::run_failure::RUN_FAILURE_NOTICE;

fn text(daemon: &FakeClaudeDaemon, prompt: &AgentChatPromptSaved) -> String {
    daemon
        .transcript(prompt)
        .into_iter()
        .filter(|event| !event.is_partial)
        .map(|event| event.text)
        .collect()
}

#[test]
fn send_now_resumes_the_switched_runs_claude_session_with_its_own_context() {
    let mut daemon = FakeClaudeDaemon::start();
    let (conversation, run) = daemon.switched_conversation("switched");
    let active = daemon.prompt(
        &conversation,
        "Remember CODE-PELICAN, then write LONG",
        Send,
    );
    daemon.drive_until("streamed output", |daemon| {
        daemon.transcript(&active).len() > 3
    });
    let queued = daemon.prompt(&conversation, "Which code did I give you?", Queue);
    daemon.drive_until("queued behind the active turn", |_| true);
    assert_eq!(daemon.phase(&queued), DurableTurnPhase::Active);

    daemon
        .router
        .interrupt_run(AgentChatProvider::Claude, &run.0)
        .unwrap();
    daemon.drive_until("queued prompt settles", |daemon| {
        daemon.phase(&queued).is_terminal()
    });

    assert_eq!(daemon.phase(&active), DurableTurnPhase::Interrupted);
    assert_eq!(daemon.phase(&queued), DurableTurnPhase::Completed);
    assert!(text(&daemon, &queued).contains("recall: CODE-PELICAN"));
    let session = daemon.bound_session(&run);
    assert_eq!(daemon.sessions(), std::slice::from_ref(&session));
    let launches = daemon.launches();
    assert_eq!(launches.len(), 2);
    assert!(
        launches
            .iter()
            .all(|argv| argv.windows(2).any(|pair| pair == ["--effort", "medium"]))
    );
    assert!(!launches[0].contains(&"--resume".to_owned()));
    assert!(
        launches[1]
            .windows(2)
            .any(|pair| pair[0] == "--resume" && pair[1] == session)
    );
}

#[test]
fn interrupt_while_permission_is_pending_settles_and_accepts_the_next_prompt() {
    let mut daemon = FakeClaudeDaemon::start();
    let (conversation, run) = daemon.switched_conversation("permission");
    let waiting = daemon.prompt(&conversation, "Remember CODE-HERON. PERMISSION", Send);
    daemon.drive_until("durable pending permission", |daemon| {
        daemon
            .ledger
            .pending_permission(&conversation, &run)
            .unwrap()
            .is_some()
    });

    daemon
        .router
        .interrupt_run(AgentChatProvider::Claude, &run.0)
        .unwrap();
    daemon.drive_until("interrupted turn", |daemon| {
        daemon.phase(&waiting).is_terminal()
    });

    assert_eq!(daemon.phase(&waiting), DurableTurnPhase::Interrupted);
    assert!(
        daemon
            .ledger
            .pending_permission(&conversation, &run)
            .unwrap()
            .is_none()
    );
    let projection = daemon.projection(&conversation);
    assert!(projection.iter().any(|event| {
        event.payload["activity"]["type"] == "decisionSettled"
            && event.payload["turnId"] == waiting.message.turn_id.as_str()
    }));
    let next = daemon.prompt(&conversation, "Which code did I give you?", Send);
    daemon.drive_until("next prompt", |daemon| daemon.phase(&next).is_terminal());
    assert_eq!(daemon.phase(&next), DurableTurnPhase::Completed);
    assert!(text(&daemon, &next).contains("recall: CODE-HERON"));
}

#[test]
fn a_run_whose_session_cannot_be_recorded_fails_alone_while_gentd_keeps_serving() {
    let mut daemon = FakeClaudeDaemon::start();
    let (broken, broken_run) = daemon.switched_conversation("broken");
    let (healthy, _) = daemon.conversation("healthy");
    let seed = daemon.prompt(&broken, "Remember CODE-OTTER", Send);
    daemon.drive_until("seed turn", |daemon| daemon.phase(&seed).is_terminal());
    let session = daemon.bound_session(&broken_run);

    let rotated = daemon.prompt(&broken, "ROTATE", Send);
    daemon.drive_until("rotated turn", |daemon| {
        daemon.phase(&rotated).is_terminal()
    });

    assert_eq!(daemon.phase(&rotated), DurableTurnPhase::Failed);
    assert_eq!(daemon.bound_session(&broken_run), session);
    let follow = TurnFollowService::read(
        &daemon.ledger,
        &TurnFollowRequest {
            conversation_id: broken.0.clone(),
            run_id: broken_run.0.clone(),
            turn_id: rotated.message.turn_id.clone(),
            after_cursor: 0,
            expected_host_epoch: daemon.epoch,
            limit: 100,
        },
    )
    .unwrap();
    assert_eq!(follow.terminal.unwrap().phase, DurableTurnPhase::Failed);
    assert!(follow.events.iter().any(|event| {
        event.kind == NormalizedTranscriptKind::Notice && event.text == RUN_FAILURE_NOTICE
    }));
    let projection = daemon.projection(&broken);
    let failure = projection
        .iter()
        .find(|event| event.payload["lifecycle"]["event"]["type"] == "providerFailure")
        .expect("the failed turn carries a typed failure");
    assert_eq!(
        failure.payload["lifecycle"]["event"]["classification"],
        "lifecycle"
    );
    assert!(projection.iter().any(|event| {
        event.payload["activity"]["type"] == "terminal"
            && event.payload["activity"]["phase"] == "failed"
            && event.payload["turnId"] == rotated.message.turn_id.as_str()
    }));
    assert!(
        !serde_json::to_string(&projection)
            .unwrap()
            .contains(&session)
    );

    let hello = daemon.prompt(&healthy, "hello", Send);
    daemon.drive_until("other conversation", |daemon| {
        daemon.phase(&hello).is_terminal()
    });
    assert_eq!(daemon.phase(&hello), DurableTurnPhase::Completed);
    let recall = daemon.prompt(&broken, "Which code did I give you?", Send);
    daemon.drive_until("recovered run", |daemon| {
        daemon.phase(&recall).is_terminal()
    });
    assert_eq!(daemon.phase(&recall), DurableTurnPhase::Completed);
    assert!(text(&daemon, &recall).contains("recall: CODE-OTTER"));
}
