use gent_types::{ConversationActivityFact, PlanArtifact, PlanStatus};
use ratatui::{
    style::{Color, Modifier, Style},
    text::Line,
};

use super::state::UiState;

const MAX_PLAN_LINES: usize = 40;

pub(super) fn plan_lines(state: &UiState) -> Vec<Line<'static>> {
    let Some(plan) = state
        .selected_activity()
        .iter()
        .rev()
        .find_map(|fact| match fact {
            ConversationActivityFact::PlanUpdated { plan, .. } => Some(plan),
            _ => None,
        })
    else {
        return Vec::new();
    };
    if !matches!(plan.status, PlanStatus::Draft | PlanStatus::ReadyForReview) {
        return vec![Line::styled(
            format!(
                "Plan revision {} · {}",
                plan.revision.0,
                status_label(plan.status)
            ),
            Style::default().fg(Color::DarkGray),
        )];
    }
    let mut lines = vec![Line::styled(
        format!(
            "Plan · revision {} · {}",
            plan.revision.0,
            status_label(plan.status)
        ),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )];
    lines.extend(
        plan.content
            .lines()
            .take(MAX_PLAN_LINES)
            .map(|line| Line::from(format!("  {line}"))),
    );
    if plan.content.lines().count() > MAX_PLAN_LINES {
        lines.push(Line::from("  …"));
    }
    lines.extend(
        actions(plan)
            .into_iter()
            .map(|action| Line::styled(action, Style::default().fg(Color::Cyan))),
    );
    lines.push(Line::default());
    lines
}

fn actions(plan: &PlanArtifact) -> Vec<String> {
    let conversation = &plan.conversation_id.0;
    vec![
        format!("Approve: gent plan start --conversation-id {conversation}"),
        format!("Reject:  gent plan reject --conversation-id {conversation}"),
        format!("Review:  gent plan review --conversation-id {conversation}"),
    ]
}

const fn status_label(status: PlanStatus) -> &'static str {
    match status {
        PlanStatus::Draft => "draft",
        PlanStatus::ReadyForReview => "ready for review",
        PlanStatus::Approved => "approved",
        PlanStatus::Rejected => "rejected",
        PlanStatus::Superseded => "superseded",
        PlanStatus::TerminallyFailed => "failed",
    }
}
