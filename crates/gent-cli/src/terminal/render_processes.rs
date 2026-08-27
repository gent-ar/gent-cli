use std::collections::BTreeMap;

use gent_types::{ActivityWorkKind, ConversationActivityFact, WorkPhase};
use ratatui::{
    style::{Color, Style},
    text::Line,
};

pub(super) fn process_lines(facts: &[ConversationActivityFact]) -> Vec<Line<'static>> {
    let mut phases = BTreeMap::new();
    for (index, fact) in facts.iter().enumerate() {
        if let ConversationActivityFact::WorkPhase {
            work_id,
            kind: ActivityWorkKind::Command,
            phase,
            ..
        } = fact
        {
            phases.insert(work_id, (index, phase));
        }
    }
    let mut phases = phases.into_iter().collect::<Vec<_>>();
    phases.sort_by_key(|(_, (index, _))| std::cmp::Reverse(*index));
    phases
        .into_iter()
        .take(6)
        .map(|(work_id, (_, phase))| {
            Line::styled(
                format!("Process · {} · {}", clip(work_id), phase_name(phase)),
                Style::default().fg(phase_color(phase)),
            )
        })
        .collect()
}

fn phase_name(phase: &WorkPhase) -> &'static str {
    match phase {
        WorkPhase::Pending => "queued",
        WorkPhase::Running => "running",
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
    use gent_types::{
        ActivityWorkKind, ConversationActivityFact, ConversationActivityScope, HostEpoch, WorkPhase,
    };

    use super::process_lines;

    #[test]
    fn process_activity_keeps_the_latest_phase_for_each_process() {
        let scope = || ConversationActivityScope {
            conversation_id: "conversation".into(),
            run_id: "run".into(),
            turn_id: "turn".into(),
            host_epoch: HostEpoch(1),
            cursor: 1,
        };
        let lines = process_lines(&[
            ConversationActivityFact::WorkPhase {
                scope: scope(),
                work_id: "command-1".into(),
                kind: ActivityWorkKind::Command,
                phase: WorkPhase::Running,
            },
            ConversationActivityFact::WorkPhase {
                scope: scope(),
                work_id: "command-1".into(),
                kind: ActivityWorkKind::Command,
                phase: WorkPhase::Done,
            },
        ]);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].to_string().contains("completed"));
    }

    #[test]
    fn process_activity_uses_activity_order_not_identifier_order() {
        let scope = |cursor| ConversationActivityScope {
            conversation_id: "conversation".into(),
            run_id: "run".into(),
            turn_id: "turn".into(),
            host_epoch: HostEpoch(1),
            cursor,
        };
        let lines = process_lines(&[
            ConversationActivityFact::WorkPhase {
                scope: scope(1),
                work_id: "zebra".into(),
                kind: ActivityWorkKind::Command,
                phase: WorkPhase::Done,
            },
            ConversationActivityFact::WorkPhase {
                scope: scope(2),
                work_id: "alpha".into(),
                kind: ActivityWorkKind::Command,
                phase: WorkPhase::Running,
            },
        ]);
        assert!(lines[0].to_string().contains("alpha"));
        assert!(lines[1].to_string().contains("zebra"));
    }
}
