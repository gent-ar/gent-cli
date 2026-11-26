use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    ConversationActivityLedger, MAX_CONVERSATION_ACTIVITY_PAGE_FACTS, PolicyLedger,
    RunLifecycleFactLedger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationId, Containment, ConversationActivityFact, ConversationWorkStatus,
    NormalizedLifecycleSignal, PermissionMode, PolicyRecord, PolicyScope, ToolActivity, ToolPhase,
    TurnPhase,
};

use super::claude_warm_pool_tests::Pool as ClaudePool;
use super::codex_warm_pool_tests::Pool as CodexPool;

const COMMAND: &str = "toolu_background_command";

fn command(phase: ToolPhase) -> PublicWireFact {
    PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity {
        activity: ToolActivity {
            tool_use_id: COMMAND.into(),
            tool_name: "Bash".into(),
            phase,
            input: None,
            output_digest: None,
            parent_tool_use_id: None,
        },
    })
}

fn launched_in_background(run_id: &str) -> Vec<PublicWireFact> {
    vec![
        PublicWireFact::SessionStarted {
            provider_session_id: format!("session-{run_id}"),
        },
        command(ToolPhase::Started),
        command(ToolPhase::Background),
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::Ready,
        }),
    ]
}

fn command_phases(
    ledger: &SqliteLedger,
    conversation: &AgentChatConversationId,
    run_id: &str,
) -> Vec<ToolPhase> {
    ledger
        .read_conversation_activity_page(&conversation.0, run_id, 0, MAX_CONVERSATION_ACTIVITY_PAGE_FACTS)
        .unwrap()
        .facts
        .into_iter()
        .filter_map(|fact| match fact {
            ConversationActivityFact::ToolActivity { activity, .. }
                if activity.tool_use_id == COMMAND =>
            {
                Some(activity.phase)
            }
            _ => None,
        })
        .collect()
}

fn command_work(ledger: &SqliteLedger, run_id: &str) -> ConversationWorkStatus {
    ledger
        .run_live_status(run_id)
        .unwrap()
        .unwrap()
        .status
        .command_work
}

fn claude_emit(pool: &mut ClaudePool, run_id: &str, effects: Vec<ClaudeRunnerEffect>) {
    pool.runner
        .0
        .lock()
        .unwrap()
        .effects
        .entry(run_id.into())
        .or_default()
        .push_back(effects);
    pool.host.tick().unwrap();
}

fn claude_launch_in_background(pool: &mut ClaudePool, run_id: &str) {
    let effects = launched_in_background(run_id)
        .into_iter()
        .map(ClaudeRunnerEffect::Fact)
        .collect();
    claude_emit(pool, run_id, effects);
}

fn codex_emit(pool: &mut CodexPool, run_id: &str, effects: Vec<CodexRunnerEffect>) {
    pool.runner
        .state
        .lock()
        .unwrap()
        .run_effects
        .entry(run_id.to_owned())
        .or_default()
        .push_back(effects);
    pool.host.tick().unwrap();
}

fn workspace_policy(revision: u64, mode: PermissionMode) -> PolicyRecord {
    PolicyRecord {
        policy_id: format!("policy-a-{revision}"),
        workspace_id: "workspace-a".into(),
        scope: PolicyScope::ProviderPermissions,
        revision,
        mode,
        containment: Containment::Unsandboxed,
        allowed_tools: vec![],
        allowed_categories: vec![],
        allowed_commands: Vec::new(),
    }
}

#[test]
fn a_permission_change_that_restarts_claude_settles_only_that_runs_background_command() {
    let mut pool = ClaudePool::new(4);
    let first = pool.conversation("a");
    let second = pool.conversation("b");
    pool.ledger
        .save_policy(&workspace_policy(1, PermissionMode::AskEveryTime))
        .unwrap();
    let first_run = pool.send(&first, "a1");
    claude_launch_in_background(&mut pool, &first_run);
    let second_run = pool.send(&second, "b1");
    claude_launch_in_background(&mut pool, &second_run);
    assert_eq!(
        command_phases(&pool.ledger, &first, &first_run),
        [ToolPhase::Started, ToolPhase::Background]
    );
    assert_eq!(
        command_work(&pool.ledger, &first_run),
        ConversationWorkStatus::Waiting
    );

    pool.ledger
        .save_policy(&workspace_policy(2, PermissionMode::AutoAcceptEdits))
        .unwrap();
    pool.send(&first, "a2");

    assert_eq!(
        pool.runner.0.lock().unwrap().releases,
        std::slice::from_ref(&first_run)
    );
    assert_eq!(
        command_phases(&pool.ledger, &first, &first_run),
        [
            ToolPhase::Started,
            ToolPhase::Background,
            ToolPhase::Interrupted,
        ]
    );
    assert_eq!(
        command_work(&pool.ledger, &first_run),
        ConversationWorkStatus::None
    );
    assert_eq!(
        command_phases(&pool.ledger, &second, &second_run),
        [ToolPhase::Started, ToolPhase::Background]
    );
    assert_eq!(
        command_work(&pool.ledger, &second_run),
        ConversationWorkStatus::Waiting
    );
}

#[test]
fn a_claude_process_that_exits_settles_the_background_command_it_was_running() {
    let mut pool = ClaudePool::new(4);
    let conversation = pool.conversation("a");
    let run_id = pool.send(&conversation, "a1");
    claude_launch_in_background(&mut pool, &run_id);
    assert_eq!(
        command_work(&pool.ledger, &run_id),
        ConversationWorkStatus::Waiting
    );

    claude_emit(
        &mut pool,
        &run_id,
        vec![ClaudeRunnerEffect::Exited { code: Some(0) }],
    );

    assert_eq!(
        command_phases(&pool.ledger, &conversation, &run_id),
        [
            ToolPhase::Started,
            ToolPhase::Background,
            ToolPhase::Failed,
        ]
    );
    assert_eq!(
        command_work(&pool.ledger, &run_id),
        ConversationWorkStatus::None
    );
}

#[test]
fn a_codex_process_that_exits_settles_the_background_command_it_was_running() {
    let mut pool = CodexPool::new(4);
    let conversation = pool.conversation("a");
    let run_id = pool.send(&conversation, "a1");
    let effects = launched_in_background(&run_id)
        .into_iter()
        .map(CodexRunnerEffect::Fact)
        .collect();
    codex_emit(&mut pool, &run_id, effects);
    assert_eq!(
        command_work(&pool.ledger, &run_id),
        ConversationWorkStatus::Waiting
    );

    codex_emit(
        &mut pool,
        &run_id,
        vec![CodexRunnerEffect::Exited { code: Some(0) }],
    );

    assert_eq!(
        command_phases(&pool.ledger, &conversation, &run_id),
        [
            ToolPhase::Started,
            ToolPhase::Background,
            ToolPhase::Failed,
        ]
    );
    assert_eq!(
        command_work(&pool.ledger, &run_id),
        ConversationWorkStatus::None
    );
}
