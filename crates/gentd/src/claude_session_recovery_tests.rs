use gent_types::{
    AgentChatPromptDisposition::Send, AgentChatPromptSaved, DurableTurnPhase,
    NormalizedTranscriptKind, PROVIDER_SESSION_RECOVERED_NOTICE,
};

use super::fake_cli::FakeClaudeDaemon;

fn settled(daemon: &mut FakeClaudeDaemon, prompt: &AgentChatPromptSaved) -> DurableTurnPhase {
    daemon.drive_until("prompt settles", |daemon| {
        daemon.phase(prompt).is_terminal()
    });
    daemon.phase(prompt)
}

fn events(
    daemon: &FakeClaudeDaemon,
    prompt: &AgentChatPromptSaved,
    kind: NormalizedTranscriptKind,
) -> Vec<String> {
    daemon
        .transcript(prompt)
        .into_iter()
        .filter(|event| event.kind == kind && !event.is_partial)
        .map(|event| event.text)
        .collect()
}

fn launches_with(daemon: &FakeClaudeDaemon, flag: &str, session: &str) -> usize {
    daemon
        .launches()
        .iter()
        .filter(|argv| argv.windows(2).any(|pair| pair == [flag, session]))
        .count()
}

fn idle_released_session(
    daemon: &mut FakeClaudeDaemon,
    key: &str,
) -> (AgentChatPromptSaved, String) {
    let (conversation, run) = daemon.conversation(key);
    let seed = daemon.prompt(&conversation, "Remember CODE-OSPREY", Send);
    assert_eq!(settled(daemon, &seed), DurableTurnPhase::Completed);
    let session = daemon.bound_session(&run);
    let (other, _) = daemon.conversation(&format!("{key}-other"));
    let unrelated = daemon.prompt(&other, "hello", Send);
    assert_eq!(settled(daemon, &unrelated), DurableTurnPhase::Completed);
    (seed, session)
}

#[test]
fn a_lost_claude_session_is_recreated_under_its_bound_identity_from_gent_history() {
    let mut daemon = FakeClaudeDaemon::start();
    let (seed, session) = idle_released_session(&mut daemon, "lost");
    daemon.forget_session(&session);

    let conversation = gent_types::AgentChatConversationId(seed.message.conversation_id.clone());
    let next = daemon.prompt(&conversation, "Which code did I give you?", Send);

    assert_eq!(settled(&mut daemon, &next), DurableTurnPhase::Completed);
    assert_eq!(
        events(&daemon, &next, NormalizedTranscriptKind::Notice),
        [PROVIDER_SESSION_RECOVERED_NOTICE]
    );
    let replies = events(&daemon, &next, NormalizedTranscriptKind::AssistantMessage);
    assert_eq!(replies.len(), 1);
    assert!(replies[0].contains("recall: CODE-OSPREY"), "{replies:?}");
    assert_eq!(
        events(&daemon, &next, NormalizedTranscriptKind::UserMessage).len(),
        1
    );
    assert_eq!(daemon.bound_session(&next.run_id), session);
    assert_eq!(launches_with(&daemon, "--resume", &session), 1);
    assert_eq!(launches_with(&daemon, "--session-id", &session), 1);
    assert_eq!(daemon.session_prompts(&session), 1);

    let after = daemon.prompt(&conversation, "Which code did I give you now?", Send);
    assert_eq!(settled(&mut daemon, &after), DurableTurnPhase::Completed);
    assert!(events(&daemon, &after, NormalizedTranscriptKind::Notice).is_empty());
}

#[test]
fn a_resume_that_fails_after_spending_api_time_is_a_genuine_failure_and_is_not_recreated() {
    let mut daemon = FakeClaudeDaemon::start();
    let (seed, session) = idle_released_session(&mut daemon, "auth");
    daemon.fail_resumes_with_an_api_error();

    let conversation = gent_types::AgentChatConversationId(seed.message.conversation_id.clone());
    let next = daemon.prompt(&conversation, "Which code did I give you?", Send);

    assert_eq!(settled(&mut daemon, &next), DurableTurnPhase::Failed);
    assert!(
        !events(&daemon, &next, NormalizedTranscriptKind::Notice)
            .iter()
            .any(|notice| notice == PROVIDER_SESSION_RECOVERED_NOTICE)
    );
    assert_eq!(launches_with(&daemon, "--resume", &session), 1);
    assert_eq!(launches_with(&daemon, "--session-id", &session), 0);
}
