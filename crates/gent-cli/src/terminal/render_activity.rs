use std::collections::{BTreeMap, HashSet};

use gent_types::{ActivityWorkKind, ConversationActivityFact, ToolPhase};
use ratatui::{
    style::{Color, Style},
    text::Line,
};

pub(super) fn counts(facts: &[ConversationActivityFact]) -> (usize, usize, usize) {
    let mut tools = BTreeMap::new();
    let mut subagents = HashSet::new();
    let mut subagent_phases = BTreeMap::new();
    let mut processes = BTreeMap::new();
    for fact in facts {
        match fact {
            ConversationActivityFact::ToolActivity { activity, .. } => {
                tools.insert(&activity.tool_use_id, &activity.phase);
            }
            ConversationActivityFact::SubagentStarted { child_id, .. } => {
                subagents.insert(child_id);
            }
            ConversationActivityFact::WorkPhase {
                work_id,
                kind: ActivityWorkKind::Subagent,
                phase,
                ..
            } => {
                subagent_phases.insert(work_id, phase);
            }
            ConversationActivityFact::WorkPhase {
                work_id,
                kind: ActivityWorkKind::Command,
                phase,
                ..
            } => {
                processes.insert(work_id, phase);
            }
            _ => {}
        }
    }
    (
        tools
            .values()
            .filter(|phase| matches!(phase, ToolPhase::Started | ToolPhase::WaitingPermission))
            .count(),
        subagents
            .iter()
            .filter(|child_id| {
                subagent_phases
                    .get(*child_id)
                    .is_none_or(|phase| phase.is_live())
            })
            .count(),
        processes.values().filter(|phase| phase.is_live()).count(),
    )
}

pub(super) fn timeline_lines(facts: &[ConversationActivityFact]) -> Vec<Line<'static>> {
    let mut facts = facts.iter().collect::<Vec<_>>();
    facts.sort_by_key(|fact| fact.scope().cursor);
    let start = facts.len().saturating_sub(24);
    facts[start..].iter().map(|fact| fact_line(fact)).collect()
}

#[path = "render_activity_links.rs"]
mod links;

fn fact_line(fact: &ConversationActivityFact) -> Line<'static> {
    let cursor = fact.scope().cursor;
    let (text, color) = match fact {
        ConversationActivityFact::TurnStarted { .. } => ("Turn started".into(), Color::Cyan),
        ConversationActivityFact::ContextUsage {
            used_tokens,
            window_tokens,
            ..
        } => (
            window_tokens.map_or_else(
                || format!("Context · {used_tokens} tokens"),
                |window| format!("Context · {used_tokens}/{window} tokens"),
            ),
            Color::DarkGray,
        ),
        ConversationActivityFact::TokenUsage { usage, .. } => {
            (format!("Tokens · {} used", usage.total()), Color::DarkGray)
        }
        ConversationActivityFact::RootActivity { activity, .. } => {
            (format!("Gent · {activity:?}"), Color::Cyan)
        }
        ConversationActivityFact::RootPhase { phase, .. } => {
            (format!("Turn · {phase:?}"), turn_color(phase))
        }
        ConversationActivityFact::WorkPhase {
            work_id,
            kind,
            phase,
            ..
        } => (
            format!("{} · {} · {phase:?}", work_name(*kind), clip(work_id)),
            work_color(phase),
        ),
        ConversationActivityFact::ToolActivity { activity, .. } => (
            format!(
                "Tool · {} · {}",
                activity.tool_name,
                tool_phase_name(&activity.phase)
            ),
            tool_color(&activity.phase),
        ),
        ConversationActivityFact::ToolOutput { text, .. } => (clip(text), Color::DarkGray),
        ConversationActivityFact::SubagentStarted { child_id, .. } => (
            format!("Subagent · {} · started", clip(child_id)),
            Color::Cyan,
        ),
        ConversationActivityFact::DecisionPending { decision_id, .. } => (
            format!("Permission · {} · requested", clip(decision_id)),
            Color::Yellow,
        ),
        ConversationActivityFact::DecisionSettled { decision_id, .. } => (
            format!("Permission · {} · settled", clip(decision_id)),
            Color::Green,
        ),
        ConversationActivityFact::PromptQueued { .. } => ("Prompt queued".into(), Color::DarkGray),
        ConversationActivityFact::PromptHeld { reason, .. } => (
            format!("Prompt held · {}", hold_reason_name(*reason)),
            Color::Yellow,
        ),
        ConversationActivityFact::PromptReleased { .. } => ("Prompt released".into(), Color::Cyan),
        ConversationActivityFact::PromptCanceled { .. } => {
            ("Prompt canceled".into(), Color::DarkGray)
        }
        ConversationActivityFact::PromptSteerRequested { .. } => {
            ("Steer requested · awaiting provider".into(), Color::Yellow)
        }
        ConversationActivityFact::PromptSteerRejected { reason, .. }
            if reason == "deliveryUnprovable" =>
        {
            (
                "Steer delivery could not be confirmed; automatic replay blocked".into(),
                Color::Yellow,
            )
        }
        ConversationActivityFact::PromptSteerRejected { .. } => (
            "Steer rejected; prompt remains queued".into(),
            Color::Yellow,
        ),
        ConversationActivityFact::PromptSteered { .. } => {
            ("Prompt steered into the running turn".into(), Color::Cyan)
        }
        ConversationActivityFact::InterruptRequested { .. } => {
            ("Interrupt requested".into(), Color::Yellow)
        }
        ConversationActivityFact::Recovered { .. } => ("Recovered".into(), Color::Cyan),
        ConversationActivityFact::Terminal {
            cause: Some(gent_types::TurnTerminalCause::Steered),
            ..
        } => ("Turn ended · Steered".into(), Color::Cyan),
        ConversationActivityFact::Terminal {
            cause: Some(gent_types::TurnTerminalCause::ProviderSessionUnavailable),
            ..
        } => (
            "Turn failed · provider session lost · /continue resends it from saved history".into(),
            Color::Red,
        ),
        ConversationActivityFact::Terminal { phase, .. } => {
            (format!("Turn ended · {phase:?}"), turn_color(phase))
        }
        ConversationActivityFact::GoalUpdated { goal, .. } => (
            format!("Goal · {:?} · {}", goal.status, clip(&goal.objective)),
            Color::Magenta,
        ),
        ConversationActivityFact::PlanUpdated { plan, .. } => (
            format!("Plan · revision {} · {:?}", plan.revision.0, plan.status),
            Color::Magenta,
        ),
        ConversationActivityFact::ConversationCreated { .. }
        | ConversationActivityFact::CreatedByConversation { .. }
        | ConversationActivityFact::ConversationMessageSent { .. }
        | ConversationActivityFact::ConversationWaitSettled { .. } => links::text(fact),
    };
    Line::styled(format!("#{cursor}  {text}"), Style::default().fg(color))
}

fn work_name(kind: ActivityWorkKind) -> &'static str {
    match kind {
        ActivityWorkKind::Command => "Process",
        ActivityWorkKind::Subagent => "Subagent",
    }
}

fn tool_phase_name(phase: &ToolPhase) -> &'static str {
    match phase {
        ToolPhase::Started => "running",
        ToolPhase::WaitingPermission => "needs permission",
        ToolPhase::Completed => "completed",
        ToolPhase::Failed => "failed",
    }
}

fn tool_color(phase: &ToolPhase) -> Color {
    match phase {
        ToolPhase::Started => Color::Cyan,
        ToolPhase::WaitingPermission => Color::Yellow,
        ToolPhase::Completed => Color::Green,
        ToolPhase::Failed => Color::Red,
    }
}

fn work_color(phase: &gent_types::WorkPhase) -> Color {
    match phase {
        gent_types::WorkPhase::Pending | gent_types::WorkPhase::Running => Color::Cyan,
        gent_types::WorkPhase::WaitingPermission => Color::Yellow,
        gent_types::WorkPhase::Done => Color::Green,
        gent_types::WorkPhase::Failed | gent_types::WorkPhase::Interrupted => Color::Red,
    }
}

const fn hold_reason_name(reason: gent_types::PromptHoldReason) -> &'static str {
    match reason {
        gent_types::PromptHoldReason::ModelDownload => "downloading model",
        gent_types::PromptHoldReason::ProviderInstall => "provider install required",
    }
}

fn turn_color(phase: &gent_types::TurnPhase) -> Color {
    match phase {
        gent_types::TurnPhase::Processing
        | gent_types::TurnPhase::Compacting
        | gent_types::TurnPhase::Ready => Color::Cyan,
        gent_types::TurnPhase::WaitingPermission
        | gent_types::TurnPhase::WaitingQuestion
        | gent_types::TurnPhase::Cancelled => Color::Yellow,
        gent_types::TurnPhase::Interrupted
        | gent_types::TurnPhase::Dead
        | gent_types::TurnPhase::Failed => Color::Red,
    }
}

fn clip(value: &str) -> String {
    let count = value.chars().count();
    let mut clipped = value.chars().take(36).collect::<String>();
    if count > 36 {
        clipped.push('…');
    }
    clipped
}

#[cfg(test)]
#[path = "render_activity_tests.rs"]
mod tests;
