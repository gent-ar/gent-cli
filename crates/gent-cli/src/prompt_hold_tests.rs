use gent_protocol::{
    DependencyAction, DependencyProvider, ProviderInstallReview, ProviderPackageReview,
};
use gent_types::{
    ConversationActivityFact, ConversationActivityScope, HostEpoch, PromptHoldReason,
};

use super::{InstallHold, current_hold, install_lines};

fn scope(run_id: &str, turn_id: &str, cursor: u64) -> ConversationActivityScope {
    ConversationActivityScope {
        conversation_id: "conversation-1".into(),
        run_id: run_id.into(),
        turn_id: turn_id.into(),
        host_epoch: HostEpoch(1),
        cursor,
    }
}

fn held(run_id: &str, turn_id: &str, cursor: u64, receipt_id: &str) -> ConversationActivityFact {
    ConversationActivityFact::PromptHeld {
        scope: scope(run_id, turn_id, cursor),
        message_id: format!("message-{cursor}"),
        receipt_id: receipt_id.into(),
        reason: PromptHoldReason::ProviderInstall,
    }
}

fn released(turn_id: &str, cursor: u64) -> ConversationActivityFact {
    ConversationActivityFact::PromptReleased {
        scope: scope("run-1", turn_id, cursor),
        message_id: format!("message-{cursor}"),
    }
}

#[test]
fn the_current_hold_is_the_latest_unreleased_hold_of_the_run() {
    let facts = [
        held("run-1", "turn-1", 1, "receipt-1"),
        released("turn-1", 2),
        held("run-1", "turn-2", 3, "receipt-2"),
        released("turn-other", 4),
        held("run-other", "turn-3", 5, "receipt-3"),
    ];
    let hold = current_hold(&facts, "run-1").unwrap();
    assert_eq!(hold.receipt_id, "receipt-2");
    assert_eq!(hold.reason, PromptHoldReason::ProviderInstall);
    assert!(current_hold(&facts[..2], "run-1").is_none());
}

pub(crate) fn claude_install_hold() -> InstallHold {
    InstallHold {
        conversation_id: "conversation-1".into(),
        run_id: "run-1".into(),
        prompt_receipt_id: "receipt-2".into(),
        review: ProviderInstallReview::reviewed(
            DependencyProvider::Claude,
            DependencyAction::Install,
            "Install Claude Code from npm",
            true,
            ProviderPackageReview {
                package_name: "@anthropic-ai/claude-code".into(),
                version: "2.1.0".into(),
                integrity: "sha512-abc".into(),
                package_policy_digest_sha256: "policy".into(),
            },
        ),
    }
}

#[test]
fn an_install_hold_prints_the_package_and_ready_to_run_consent_commands() {
    let hold = claude_install_hold();
    let digest = hold.review.reviewed_plan_digest.clone();
    let provision = format!(
        "gent --data-dir /tmp/g provider provision --conversation-id conversation-1 --run-id run-1 --prompt-receipt-id receipt-2 --reviewed-plan-digest {digest}"
    );
    assert_eq!(
        install_lines(&hold, "gent --data-dir /tmp/g"),
        [
            "! This prompt is waiting for you to install Claude".to_owned(),
            "    @anthropic-ai/claude-code@2.1.0 · Install Claude Code from npm".to_owned(),
            format!("  Install: {provision} --consent"),
            format!("  Decline: {provision}"),
        ]
    );
}
