use std::path::PathBuf;

use gent_protocol::{
    DependencyAction, DependencyProvider, ProviderInstallReview, ProviderReadinessFrame,
};
use gent_types::{ConversationActivityFact, PromptHoldReason};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PromptHold {
    pub(crate) reason: PromptHoldReason,
    pub(crate) receipt_id: String,
    turn_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InstallHold {
    pub(crate) conversation_id: String,
    pub(crate) run_id: String,
    pub(crate) prompt_receipt_id: String,
    pub(crate) review: ProviderInstallReview,
}

pub(crate) fn hold_after(
    hold: Option<PromptHold>,
    fact: &ConversationActivityFact,
) -> Option<PromptHold> {
    match fact {
        ConversationActivityFact::PromptHeld {
            scope,
            receipt_id,
            reason,
            ..
        } => Some(PromptHold {
            reason: *reason,
            receipt_id: receipt_id.clone(),
            turn_id: scope.turn_id.clone(),
        }),
        ConversationActivityFact::PromptReleased { .. }
        | ConversationActivityFact::PromptCanceled { .. }
        | ConversationActivityFact::TurnStarted { .. }
        | ConversationActivityFact::Terminal { .. }
            if hold
                .as_ref()
                .is_some_and(|hold| hold.turn_id == fact.scope().turn_id) =>
        {
            None
        }
        _ => hold,
    }
}

pub(crate) fn current_hold<'a>(
    facts: impl IntoIterator<Item = &'a ConversationActivityFact>,
    run_id: &str,
) -> Option<PromptHold> {
    facts
        .into_iter()
        .filter(|fact| fact.scope().run_id == run_id)
        .fold(None, hold_after)
}

pub(crate) async fn install_review(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: &str,
    run_id: &str,
    hold: &PromptHold,
) -> Result<Option<InstallHold>, Box<dyn std::error::Error>> {
    if hold.reason != PromptHoldReason::ProviderInstall {
        return Ok(None);
    }
    let readiness =
        crate::provider_lifecycle_cli::readiness(data_dir, no_autostart, conversation_id, run_id)
            .await?;
    let ProviderReadinessFrame::Review { review, .. } = readiness else {
        return Ok(None);
    };
    Ok(Some(InstallHold {
        conversation_id: conversation_id.to_owned(),
        run_id: run_id.to_owned(),
        prompt_receipt_id: hold.receipt_id.clone(),
        review,
    }))
}

pub(crate) fn install_lines(hold: &InstallHold, command_prefix: &str) -> Vec<String> {
    let review = &hold.review;
    let provider = match review.provider {
        DependencyProvider::Claude => "Claude",
        DependencyProvider::Codex => "Codex",
    };
    let (verb, label) = match review.action {
        DependencyAction::Install => ("install", "Install:"),
        DependencyAction::Update => ("update", "Update: "),
    };
    let provision = format!(
        "{command_prefix} provider provision --conversation-id {} --run-id {} --prompt-receipt-id {} --reviewed-plan-digest {}",
        hold.conversation_id, hold.run_id, hold.prompt_receipt_id, review.reviewed_plan_digest,
    );
    vec![
        format!("! This prompt is waiting for you to {verb} {provider}"),
        format!(
            "    {}@{} · {}",
            review.package.package_name, review.package.version, review.instruction
        ),
        format!("  {label} {provision} --consent"),
        format!("  Decline: {provision}"),
    ]
}

#[cfg(test)]
#[path = "prompt_hold_tests.rs"]
pub(crate) mod tests;
