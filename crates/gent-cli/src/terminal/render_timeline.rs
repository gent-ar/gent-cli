use ratatui::{
    style::{Color, Style},
    text::Line,
};

use super::state::UiState;

pub(super) fn timeline_lines(state: &UiState) -> Vec<Line<'static>> {
    let Some(timeline) = state.selected_timeline() else {
        return Vec::new();
    };
    let mut lines = vec![timeline_summary(timeline)];
    for run in timeline.runs.iter().rev().take(3) {
        lines.push(Line::styled(
            format!(
                "Run · {} · {} · {}",
                clip(&run.run_id),
                run.provider,
                if run.parent_run_id.is_some() {
                    "fork"
                } else {
                    "root"
                }
            ),
            Style::default().fg(Color::Gray),
        ));
        for turn in run.turns.iter().rev().take(2).rev() {
            lines.push(Line::styled(
                format!("  Turn {} · {:?}", turn.sequence, turn.phase),
                Style::default().fg(Color::DarkGray),
            ));
        }
        if let Some(checkpoint) = run.checkpoints.last() {
            lines.push(Line::styled(
                format!(
                    "  Checkpoint {} · event {}",
                    checkpoint.sequence, checkpoint.event_cursor
                ),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }
    for artifact in timeline.artifacts.iter().rev().take(2) {
        lines.push(Line::styled(
            format!("Artifact · {:?} · {:?}", artifact.kind, artifact.status),
            Style::default().fg(Color::DarkGray),
        ));
    }
    lines.push(Line::default());
    lines
}

fn timeline_summary(timeline: &gent_types::ConversationTimeline) -> Line<'static> {
    let turns = timeline
        .runs
        .iter()
        .map(|run| run.turns.len())
        .sum::<usize>();
    let artifacts = timeline.artifacts.len();
    let checkpoints = timeline
        .runs
        .iter()
        .map(|run| run.checkpoints.len())
        .sum::<usize>();
    Line::styled(
        format!(
            "Timeline  ·  {} run{}  ·  {turns} turn{}  ·  {checkpoints} checkpoint{}  ·  {artifacts} artifact{}",
            timeline.runs.len(),
            plural(timeline.runs.len()),
            plural(turns),
            plural(checkpoints),
            plural(artifacts),
        ),
        Style::default().fg(Color::DarkGray),
    )
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn clip(value: &str) -> String {
    let mut clipped = value.chars().take(18).collect::<String>();
    if value.chars().count() > 18 {
        clipped.push('…');
    }
    clipped
}
