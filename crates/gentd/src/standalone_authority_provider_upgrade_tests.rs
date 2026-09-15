use gent_types::{
    AgentChatConversationId, AgentChatPromptSaved, AgentChatProvider, DurableTurnPhase,
    NormalizedTranscriptKind, PROVIDER_SESSION_RECOVERED_NOTICE,
    PROVIDER_SESSION_UNAVAILABLE_NOTICE,
};

use super::InstalledProvider;

fn remembered(
    provider: AgentChatProvider,
) -> (
    InstalledProvider,
    AgentChatConversationId,
    AgentChatPromptSaved,
) {
    let mut installed = InstalledProvider::start(provider);
    let conversation = installed.conversation();
    let seed = installed.send(&conversation, "Remember CODE-HERON");
    assert_eq!(installed.settle(&seed), DurableTurnPhase::Completed);
    (installed, conversation, seed)
}

fn upgrade_is_held_then_installed(
    installed: &mut InstalledProvider,
    conversation: &AgentChatConversationId,
) -> AgentChatPromptSaved {
    installed.sign_unreleased("2.0.0");
    let next = installed.send(conversation, "Which code did I give you?");
    installed.assert_reinstall_review(&next);
    installed.install("2.0.0");
    next
}

fn assert_recalled_on_upgraded_binary(
    installed: &InstalledProvider,
    seed: &AgentChatPromptSaved,
    next: &AgentChatPromptSaved,
) {
    assert_eq!(installed.phase(next), DurableTurnPhase::Completed);
    assert!(
        installed.reply(next).contains("CODE-HERON"),
        "{}",
        installed.reply(next)
    );
    assert_eq!(
        installed
            .transcript(next, NormalizedTranscriptKind::UserMessage)
            .len(),
        1
    );
    assert_eq!(
        installed.launched_digest(next),
        installed
            .installed
            .0
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .digest_sha256
    );
    assert_eq!(installed.events("runExecutableRebound").len(), 1);
    assert_eq!(installed.bound_session(next), installed.bound_session(seed));
}

#[test]
fn a_codex_conversation_continues_its_thread_on_the_upgraded_binary() {
    let (mut codex, conversation, seed) = remembered(AgentChatProvider::Codex);
    let next = upgrade_is_held_then_installed(&mut codex, &conversation);
    codex.wake(&next);
    codex.settle(&next);

    assert_recalled_on_upgraded_binary(&codex, &seed, &next);
    assert!(
        codex
            .transcript(&next, NormalizedTranscriptKind::Notice)
            .is_empty()
    );
    assert_eq!(codex.requests("thread/start").len(), 1);
    let resumes = codex.requests("thread/resume");
    assert_eq!(resumes.len(), 1);
    assert_eq!(resumes[0]["params"]["threadId"], codex.bound_session(&seed));
    assert_eq!(codex.requests("turn/start").len(), 2);
}

#[test]
fn a_claude_conversation_resumes_its_session_on_the_upgraded_binary() {
    let (mut claude, conversation, seed) = remembered(AgentChatProvider::Claude);
    let next = upgrade_is_held_then_installed(&mut claude, &conversation);
    claude.wake(&next);
    claude.settle(&next);

    assert_recalled_on_upgraded_binary(&claude, &seed, &next);
    assert!(
        claude
            .transcript(&next, NormalizedTranscriptKind::Notice)
            .is_empty()
    );
    let session = claude.bound_session(&seed);
    let launches = claude
        .claude_launches()
        .into_iter()
        .filter(|argv| !argv.iter().any(|flag| flag == "--no-session-persistence"))
        .collect::<Vec<_>>();
    assert_eq!(launches.len(), 2, "{launches:?}");
    assert!(
        launches[1]
            .windows(2)
            .any(|pair| pair == ["--resume", session.as_str()])
    );
}

#[test]
fn a_held_prompt_survives_a_restart_during_the_upgrade_and_runs_once_on_the_new_binary() {
    for provider in [AgentChatProvider::Codex, AgentChatProvider::Claude] {
        let (mut installed, conversation, seed) = remembered(provider);
        let next = upgrade_is_held_then_installed(&mut installed, &conversation);
        installed.restart();
        installed.wake(&next);
        installed.settle(&next);

        assert_recalled_on_upgraded_binary(&installed, &seed, &next);
        let after = installed.send(&conversation, "And which code now?");
        assert_eq!(installed.settle(&after), DurableTurnPhase::Completed);
        assert!(installed.reply(&after).contains("CODE-HERON"));
        assert_eq!(installed.events("runExecutableRebound").len(), 1);
    }
}

#[test]
fn a_prompt_pending_across_an_upgrade_and_a_restart_runs_once_on_the_new_binary() {
    for provider in [AgentChatProvider::Codex, AgentChatProvider::Claude] {
        let (mut installed, conversation, seed) = remembered(provider);
        installed.install("2.0.0");
        let next = installed.save(&conversation, "Which code did I give you?");
        crate::readiness_test_support::release(installed.state.ledger(), &next);
        installed.restart();
        installed.settle(&next);

        assert_recalled_on_upgraded_binary(&installed, &seed, &next);
    }
}

#[test]
fn a_codex_thread_the_upgraded_binary_cannot_resume_continues_once_from_saved_history() {
    let (mut codex, conversation, seed) = remembered(AgentChatProvider::Codex);
    let thread = codex.bound_session(&seed);
    let next = upgrade_is_held_then_installed(&mut codex, &conversation);
    codex.lose_thread(&thread);
    codex.wake(&next);

    assert_eq!(codex.settle(&next), DurableTurnPhase::Completed);
    assert_eq!(
        codex.transcript(&next, NormalizedTranscriptKind::Notice),
        [PROVIDER_SESSION_RECOVERED_NOTICE]
    );
    let turns = codex.requests("turn/start");
    assert!(
        turns[1]["params"]["input"]
            .to_string()
            .contains("CODE-HERON")
    );
    assert!(!codex.reply(&next).is_empty());
    assert_eq!(
        codex
            .transcript(&next, NormalizedTranscriptKind::UserMessage)
            .len(),
        1
    );
    assert_eq!(codex.requests("thread/start").len(), 2);
    assert_eq!(codex.requests("turn/start").len(), 2);
    assert_ne!(codex.bound_session(&next), thread);
    assert_eq!(codex.events("runSessionRetired").len(), 1);

    let after = codex.send(&conversation, "And which code now?");
    assert_eq!(codex.settle(&after), DurableTurnPhase::Completed);
    assert!(
        codex
            .transcript(&after, NormalizedTranscriptKind::Notice)
            .is_empty()
    );
    assert_eq!(codex.requests("thread/start").len(), 2);
}

#[test]
fn a_codex_thread_lost_without_an_upgrade_still_asks_before_continuing_from_history() {
    let (mut codex, conversation, seed) = remembered(AgentChatProvider::Codex);
    codex.restart();
    codex.lose_thread(&codex.bound_session(&seed));
    let next = codex.send(&conversation, "Which code did I give you?");

    assert_eq!(codex.settle(&next), DurableTurnPhase::Failed);
    assert_eq!(
        codex.transcript(&next, NormalizedTranscriptKind::Notice),
        [PROVIDER_SESSION_UNAVAILABLE_NOTICE]
    );
    assert_eq!(codex.requests("thread/start").len(), 1);
    assert!(codex.events("runSessionRetired").is_empty());
}

#[test]
fn a_claude_session_the_upgraded_binary_cannot_find_is_recreated_once_from_saved_history() {
    let (mut claude, conversation, seed) = remembered(AgentChatProvider::Claude);
    let session = claude.bound_session(&seed);
    let next = upgrade_is_held_then_installed(&mut claude, &conversation);
    std::fs::remove_file(
        claude
            .binary()
            .with_file_name("sessions")
            .join(format!("{session}.jsonl")),
    )
    .unwrap();
    claude.wake(&next);

    assert_eq!(claude.settle(&next), DurableTurnPhase::Completed);
    assert_eq!(
        claude.transcript(&next, NormalizedTranscriptKind::Notice),
        [PROVIDER_SESSION_RECOVERED_NOTICE]
    );
    assert!(
        claude.reply(&next).contains("CODE-HERON"),
        "{}",
        claude.reply(&next)
    );
    assert_eq!(claude.bound_session(&next), session);
}
