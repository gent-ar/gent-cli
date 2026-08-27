use std::collections::BTreeMap;

use gent_types::{ActivityWorkKind, ConversationActivityFact, WorkPhase};
use ratatui::{
    style::{Color, Style},
    text::Line,
};

pub(super) fn subagent_lines(facts: &[ConversationActivityFact]) -> Vec<Line<'static>> {
    let mut phases = BTreeMap::new();
    let mut subagents = BTreeMap::new();
    for (index, fact) in facts.iter().enumerate() {
        if let ConversationActivityFact::WorkPhase {
            work_id,
            kind: ActivityWorkKind::Subagent,
            phase,
            ..
        } = fact
        {
            phases.insert(work_id.as_str(), (index, phase));
        }
        if let ConversationActivityFact::SubagentStarted { child_id, .. } = fact {
            subagents.insert(child_id.as_str(), index);
        }
    }
    let mut subagents = subagents.into_iter().collect::<Vec<_>>();
    subagents.sort_by_key(|(child_id, started_index)| {
        std::cmp::Reverse(
            phases
                .get(child_id)
                .map_or(*started_index, |(index, _)| *index),
        )
    });
    subagents
        .into_iter()
        .take(6)
        .map(|(child_id, _)| subagent_line(child_id, phases.get(child_id).map(|(_, phase)| *phase)))
        .collect()
}

fn subagent_line(child_id: &str, phase: Option<&WorkPhase>) -> Line<'static> {
    let label = phase.map_or("started", phase_name);
    Line::styled(
        format!("Subagent · {} · {label}", clip(child_id)),
        Style::default().fg(phase.map_or(Color::Cyan, phase_color)),
    )
}

fn phase_name(phase: &WorkPhase) -> &'static str {
    match phase {
        WorkPhase::Pending => "pending",
        WorkPhase::Running => "working",
        WorkPhase::WaitingPermission => "needs permission",
        WorkPhase::Done => "completed",
        WorkPhase::Failed => "failed",
        WorkPhase::Interrupted => "interrupted",
    }
}

fn phase_color(phase: &WorkPhase) -> Color {
    match phase {
        WorkPhase::Pending | WorkPhase::Running => Color::Cyan,
        WorkPhase::WaitingPermission => Color::Yellow,
        WorkPhase::Done => Color::Green,
        WorkPhase::Failed | WorkPhase::Interrupted => Color::Red,
    }
}

fn clip(value: &str) -> String {
    let mut clipped = value.chars().take(36).collect::<String>();
    if value.chars().count() > 36 {
        clipped.push('…');
    }
    clipped
}

#[cfg(test)]
mod tests {
    use gent_types::{ConversationActivityScope, HostEpoch};

    use super::subagent_lines;

    #[test]
    fn subagent_activity_uses_activity_order_not_identifier_order() {
        let lines = subagent_lines(&[started("zebra", 1), started("alpha", 2)]);
        assert!(lines[0].to_string().contains("alpha"));
        assert!(lines[1].to_string().contains("zebra"));
    }

    fn started(child_id: &str, cursor: u64) -> gent_types::ConversationActivityFact {
        gent_types::ConversationActivityFact::SubagentStarted {
            scope: ConversationActivityScope {
                conversation_id: "conversation".into(),
                run_id: "run".into(),
                turn_id: "turn".into(),
                host_epoch: HostEpoch(1),
                cursor,
            },
            child_id: child_id.into(),
            parent_tool_use_id: "tool".into(),
        }
    }
}
