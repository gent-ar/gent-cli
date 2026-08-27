use std::collections::BTreeMap;

use gent_types::{ConversationActivityFact, ToolActivity, ToolPhase};
use ratatui::{
    style::{Color, Style},
    text::Line,
};

pub(super) fn tool_lines(facts: &[ConversationActivityFact]) -> Vec<Line<'static>> {
    let mut tools = BTreeMap::new();
    for (index, fact) in facts.iter().enumerate() {
        if let ConversationActivityFact::ToolActivity { activity, .. } = fact {
            tools.insert(activity.tool_use_id.clone(), (index, activity));
        }
    }
    let mut tools = tools.into_values().collect::<Vec<_>>();
    tools.sort_by_key(|(index, _)| std::cmp::Reverse(*index));
    tools
        .into_iter()
        .take(6)
        .map(|(_, activity)| tool_line(activity))
        .collect()
}

fn tool_line(activity: &ToolActivity) -> Line<'static> {
    Line::styled(
        format!(
            "Tool · {} · {}",
            activity.tool_name,
            phase_name(&activity.phase)
        ),
        Style::default().fg(phase_color(&activity.phase)),
    )
}

fn phase_name(phase: &ToolPhase) -> &'static str {
    match phase {
        ToolPhase::Started => "running",
        ToolPhase::WaitingPermission => "needs permission",
        ToolPhase::Completed => "completed",
        ToolPhase::Failed => "failed",
    }
}

fn phase_color(phase: &ToolPhase) -> Color {
    match phase {
        ToolPhase::Started => Color::Cyan,
        ToolPhase::WaitingPermission => Color::Yellow,
        ToolPhase::Completed => Color::Green,
        ToolPhase::Failed => Color::Red,
    }
}

#[cfg(test)]
mod tests {
    use gent_types::{ConversationActivityScope, HostEpoch, ToolActivity, ToolPhase};

    use super::tool_lines;

    #[test]
    fn tool_activity_uses_activity_order_not_identifier_order() {
        let lines = tool_lines(&[
            fact("zebra", "older", ToolPhase::Completed, 1),
            fact("alpha", "newer", ToolPhase::Started, 2),
        ]);

        assert!(lines[0].to_string().contains("newer"));
        assert!(lines[1].to_string().contains("older"));
    }

    fn fact(
        tool_use_id: &str,
        tool_name: &str,
        phase: ToolPhase,
        cursor: u64,
    ) -> gent_types::ConversationActivityFact {
        gent_types::ConversationActivityFact::ToolActivity {
            scope: ConversationActivityScope {
                conversation_id: "conversation".into(),
                run_id: "run".into(),
                turn_id: "turn".into(),
                host_epoch: HostEpoch(1),
                cursor,
            },
            activity: ToolActivity {
                tool_use_id: tool_use_id.into(),
                tool_name: tool_name.into(),
                phase,
                output_digest: None,
            },
        }
    }
}
