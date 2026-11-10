use std::collections::{BTreeMap, BTreeSet};

use gent_types::{
    AgentChatFailureCode, ConversationActivityFact, NormalizedTranscriptEvent,
    NormalizedTranscriptKind,
};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::{
    inline_tools::{ToolView, tool_lines},
    state::UiState,
};

#[derive(Clone, Debug)]
pub(crate) struct ScrollItem {
    pub(crate) key: String,
    pub(crate) done: bool,
    pub(crate) tool: bool,
    pub(crate) lines: Vec<Line<'static>>,
}

#[derive(Default)]
struct Tools {
    views: BTreeMap<String, ToolView>,
    turns: BTreeMap<String, Vec<String>>,
    outputs: BTreeMap<String, Vec<(String, String)>>,
    emitted: BTreeSet<String>,
}

impl Tools {
    fn new(facts: &[ConversationActivityFact], live: bool) -> Self {
        let mut tools = Self::default();
        let latest = facts.iter().rev().find_map(|fact| match fact {
            ConversationActivityFact::ToolActivity { scope, .. } => Some(scope.turn_id.clone()),
            _ => None,
        });
        for fact in facts {
            match fact {
                ConversationActivityFact::ToolActivity { scope, activity } => {
                    let id = activity.tool_use_id.clone();
                    let view = tools.views.entry(id.clone()).or_insert_with(|| {
                        tools
                            .turns
                            .entry(scope.turn_id.clone())
                            .or_default()
                            .push(id);
                        ToolView {
                            name: activity.tool_name.clone(),
                            settled: !live || latest.as_ref() != Some(&scope.turn_id),
                            ..ToolView::default()
                        }
                    });
                    view.phase = Some(activity.phase.clone());
                    if activity.input.is_some() {
                        view.input.clone_from(&activity.input);
                    }
                }
                ConversationActivityFact::ToolOutput {
                    scope,
                    tool_use_id,
                    text,
                    ..
                } => {
                    if let Some(view) = tools.views.get_mut(tool_use_id) {
                        view.output.push_str(text);
                    }
                    tools
                        .outputs
                        .entry(scope.turn_id.clone())
                        .or_default()
                        .push((tool_use_id.clone(), text.clone()));
                }
                _ => {}
            }
        }
        tools
    }

    fn anchor(&mut self, turn_id: &str, text: &str) -> Option<String> {
        let outputs = self.outputs.get_mut(turn_id)?;
        let index = outputs.iter().position(|(_, output)| output == text)?;
        Some(outputs.remove(index).0)
    }

    fn emit_until(&mut self, turn_id: &str, until: Option<&str>, items: &mut Vec<ScrollItem>) {
        let order = self.turns.get(turn_id).cloned().unwrap_or_default();
        for id in order {
            if self.emitted.insert(id.clone())
                && let Some(view) = self.views.get(&id)
            {
                items.push(ScrollItem {
                    key: format!("tool:{id}"),
                    done: matches!(
                        view.phase,
                        Some(gent_types::ToolPhase::Completed | gent_types::ToolPhase::Failed)
                    ) || view.settled,
                    tool: true,
                    lines: tool_lines(view),
                });
            }
            if until == Some(id.as_str()) {
                return;
            }
        }
    }
}

pub(crate) fn scroll_items(state: &UiState, width: u16) -> Vec<ScrollItem> {
    let events = state.selected_transcript();
    let live = state.turn_active() || state.selected_pending_permission().is_some();
    let queued = state
        .selected_queue()
        .into_iter()
        .map(|prompt| format!("user:{}", prompt.message_id))
        .collect::<BTreeSet<_>>();
    let mut tools = Tools::new(state.selected_activity(), live);
    let last_index = events
        .iter()
        .enumerate()
        .map(|(index, event)| (event.turn_id.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let mut items = Vec::new();
    for index in reply_order(events) {
        let event = &events[index];
        let settled = !event.is_partial
            || events
                .last()
                .is_some_and(|last| last.turn_id != event.turn_id);
        match event.kind {
            NormalizedTranscriptKind::UserMessage if queued.contains(&event.event_id) => {}
            NormalizedTranscriptKind::ToolActivity => {
                if let Some(id) = tools.anchor(&event.turn_id, &event.text) {
                    tools.emit_until(&event.turn_id, Some(&id), &mut items);
                }
            }
            NormalizedTranscriptKind::Thinking if !state.show_thinking() => {}
            _ => items.extend(text_item(
                event,
                settled,
                usize::from(width.saturating_sub(2)).max(10),
                (event.failure_code == Some(AgentChatFailureCode::ProviderAuthRequired))
                    .then(|| super::inline_view::sign_in_hint(state))
                    .flatten(),
            )),
        }
        if last_index.get(&event.turn_id) == Some(&index) {
            tools.emit_until(&event.turn_id, None, &mut items);
        }
    }
    let turns = tools.turns.keys().cloned().collect::<Vec<_>>();
    for turn in turns {
        tools.emit_until(&turn, None, &mut items);
    }
    items
}

fn reply_order(events: &[NormalizedTranscriptEvent]) -> Vec<usize> {
    let mut first_reply = BTreeMap::new();
    for (index, event) in events.iter().enumerate() {
        if event.kind != NormalizedTranscriptKind::UserMessage {
            first_reply.entry(event.turn_id.as_str()).or_insert(index);
        }
    }
    let mut deferred: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut order = Vec::with_capacity(events.len());
    for (index, event) in events.iter().enumerate() {
        order.extend(deferred.remove(&index).unwrap_or_default());
        match first_reply.get(event.turn_id.as_str()) {
            Some(reply)
                if event.kind == NormalizedTranscriptKind::UserMessage && *reply > index =>
            {
                deferred.entry(*reply).or_default().push(index);
            }
            _ => order.push(index),
        }
    }
    order
}

fn text_item(
    event: &NormalizedTranscriptEvent,
    done: bool,
    width: usize,
    hint: Option<String>,
) -> Option<ScrollItem> {
    let body = tidy(&event.text)
        .into_iter()
        .chain(hint)
        .flat_map(|line| wrap(&line, width))
        .collect::<Vec<_>>();
    if body.is_empty() {
        return None;
    }
    let (marker, marker_style, text_style) = match event.kind {
        NormalizedTranscriptKind::UserMessage => (
            "> ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        NormalizedTranscriptKind::Thinking => (
            "✻ ",
            Style::default().fg(Color::DarkGray),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        ),
        NormalizedTranscriptKind::Notice => {
            let color = match event.failure_code {
                Some(AgentChatFailureCode::ProviderAuthRequired) | None => Color::Yellow,
                Some(_) => Color::Red,
            };
            ("! ", Style::default().fg(color), Style::default().fg(color))
        }
        NormalizedTranscriptKind::Plan => {
            ("◆ ", Style::default().fg(Color::Magenta), Style::default())
        }
        _ => ("● ", Style::default(), Style::default()),
    };
    let lines = body
        .iter()
        .enumerate()
        .map(|(index, line)| {
            if line.is_empty() {
                return Line::default();
            }
            let lead = if index == 0 {
                Span::styled(marker, marker_style)
            } else {
                Span::raw("  ")
            };
            Line::from(vec![lead, Span::styled(line.clone(), text_style)])
        })
        .collect();
    Some(ScrollItem {
        key: event.event_id.clone(),
        done,
        tool: false,
        lines,
    })
}

pub(crate) fn tidy(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines().map(str::trim_end) {
        if line.is_empty() && lines.last().is_none_or(String::is_empty) {
            continue;
        }
        lines.push(line.to_owned());
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

fn wrap(line: &str, width: usize) -> Vec<String> {
    let body = line.trim_start();
    let mut current = line[..line.len() - body.len()].to_owned();
    let mut rows = Vec::new();
    for word in body.split(' ') {
        let mut word = word.to_owned();
        let used = current.trim().chars().count();
        if used > 0 && current.chars().count() + 1 + word.chars().count() > width {
            rows.push(std::mem::take(&mut current));
        } else if used > 0 {
            current.push(' ');
        }
        while current.chars().count() + word.chars().count() > width {
            let take = width.saturating_sub(current.chars().count()).max(1);
            current.extend(word.chars().take(take));
            rows.push(std::mem::take(&mut current));
            word = word.chars().skip(take).collect();
        }
        current.push_str(&word);
    }
    rows.push(current);
    rows
}

pub(crate) fn spaced(items: &[&ScrollItem], previous_tool: Option<bool>) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut previous = previous_tool;
    for item in items {
        if previous.is_some_and(|tool| !(tool && item.tool)) {
            lines.push(Line::default());
        }
        lines.extend(item.lines.iter().cloned());
        previous = Some(item.tool);
    }
    lines
}
