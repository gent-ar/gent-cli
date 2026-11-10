use std::time::Duration;

use gent_types::{AgentChatFailureCode, AgentChatProvider, DurableTurnPhase, PermissionMode};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph, Wrap},
};

use super::{
    inline_items::{ScrollItem, spaced},
    inline_live::live_lines,
    inline_popups::popup_lines,
    render::operational_context,
    render_picker::picker_widget,
    state::UiState,
};

const INPUT_ROWS: usize = 6;
const PICKER_ROWS: u16 = 10;

pub(crate) struct Composition {
    pub(crate) live: Vec<Line<'static>>,
    pub(crate) picker: bool,
    pub(crate) prompt: Vec<String>,
    pub(crate) footer: Line<'static>,
}

pub(crate) fn compose(
    state: &UiState,
    pending: &[&ScrollItem],
    previous_tool: Option<bool>,
    elapsed: Option<Duration>,
    width: u16,
) -> Composition {
    let picker = state.documents_visible || state.templates_visible || state.automations_visible;
    let live = if picker {
        Vec::new()
    } else if let Some(popup) = popup_lines(state) {
        popup
    } else {
        let mut lines = spaced(pending, previous_tool);
        let tail = live_lines(state, elapsed);
        if !lines.is_empty() && !tail.is_empty() {
            lines.push(Line::default());
        }
        lines.extend(tail);
        if let Some(notice) = state.notice().filter(|notice| {
            !(notice.is_empty() || state.turn_active() && notice.starts_with("Gent is "))
        }) {
            lines.push(Line::styled(
                notice.to_owned(),
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines
    };
    let mut live = live;
    if pending.is_empty() && !live.is_empty() {
        live.insert(0, Line::default());
    }
    Composition {
        live,
        picker,
        prompt: prompt_rows(state.input(), width.saturating_sub(6).max(8).into()),
        footer: footer(state, width),
    }
}

impl Composition {
    fn live_rows(&self, width: u16) -> u16 {
        if self.picker {
            return PICKER_ROWS;
        }
        let rows = Paragraph::new(self.live.clone())
            .wrap(Wrap { trim: false })
            .line_count(width);
        u16::try_from(rows).unwrap_or(u16::MAX)
    }

    fn prompt_height(&self) -> u16 {
        u16::try_from(self.prompt.len()).unwrap_or(1) + 2
    }

    pub(crate) fn height(&self, width: u16, screen: u16) -> u16 {
        let chrome = self.prompt_height() + 1;
        (self.live_rows(width) + chrome).min(screen.max(chrome))
    }

    #[cfg(test)]
    pub(crate) fn chrome_rows(&self) -> u16 {
        self.prompt_height() + 1
    }

    pub(crate) fn draw(&self, frame: &mut Frame, state: &UiState) {
        let area = frame.area();
        let [live, prompt, footer] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(self.prompt_height()),
            Constraint::Length(1),
        ])
        .areas(area);
        if self.picker {
            frame.render_widget(picker_widget(state), live);
        } else {
            let rows = self.live_rows(live.width);
            frame.render_widget(
                Paragraph::new(self.live.clone())
                    .wrap(Wrap { trim: false })
                    .scroll((rows.saturating_sub(live.height), 0)),
                live,
            );
        }
        self.draw_prompt(frame, prompt, state);
        frame.render_widget(Paragraph::new(self.footer.clone()), footer);
    }

    fn draw_prompt(&self, frame: &mut Frame, area: Rect, state: &UiState) {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray));
        let inner = block.inner(area);
        let lines = if state.input().is_empty() {
            vec![Line::from(vec![
                Span::styled(
                    "> ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(placeholder(state), Style::default().fg(Color::DarkGray)),
            ])]
        } else {
            self.prompt
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    let lead = if index == 0 { "> " } else { "  " };
                    Line::from(vec![
                        Span::styled(
                            lead,
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(row.clone()),
                    ])
                })
                .collect()
        };
        frame.render_widget(Paragraph::new(lines).block(block), area);
        let last = self.prompt.last().map_or(0, |row| row.chars().count());
        let row = u16::try_from(self.prompt.len().saturating_sub(1)).unwrap_or(0);
        let column = if state.input().is_empty() { 0 } else { last };
        frame.set_cursor_position(Position::new(
            inner.x + 2 + u16::try_from(column).unwrap_or(0),
            inner.y + row,
        ));
    }
}

fn placeholder(state: &UiState) -> &'static str {
    if state.selected_pending_permission().is_some() {
        "press 1-4 to answer, or type a message"
    } else if state.turn_active() {
        "type to queue a message for after this turn"
    } else {
        "Ask Gent to do something · / commands · ? shortcuts"
    }
}

fn prompt_rows(input: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for line in input.split('\n') {
        let chars = line.chars().collect::<Vec<_>>();
        if chars.is_empty() {
            rows.push(String::new());
        }
        for chunk in chars.chunks(width) {
            rows.push(chunk.iter().collect());
        }
    }
    if input.ends_with('\n') && rows.last().is_some_and(|row| !row.is_empty()) {
        rows.push(String::new());
    }
    let start = rows.len().saturating_sub(INPUT_ROWS);
    rows.split_off(start)
}

pub(crate) fn mode_label(mode: PermissionMode) -> (&'static str, Color) {
    match mode {
        PermissionMode::AskEveryTime => ("⏵ ask every time", Color::Yellow),
        PermissionMode::AutoAcceptEdits => ("⏵⏵ auto-accept edits", Color::Cyan),
        PermissionMode::Autonomous => ("⏵⏵⏵ autonomous", Color::Magenta),
        PermissionMode::Bypass => ("⚠ bypass", Color::Red),
    }
}

fn footer(state: &UiState, width: u16) -> Line<'static> {
    let selection = state.selection();
    let (mode, mode_color) = mode_label(state.permission_mode());
    let dim = Style::default().fg(Color::DarkGray);
    let mut parts: Vec<(String, Style, u8)> = vec![
        (
            crate::chat_cli::provider_name(selection.provider).to_owned(),
            dim,
            0,
        ),
        (selection.model.clone(), dim, 0),
        (format!("{:?}", selection.effort).to_lowercase(), dim, 3),
        (format!("{:?}", selection.mode).to_lowercase(), dim, 2),
        (mode.to_owned(), Style::default().fg(mode_color), 0),
    ];
    if state.mode_applies_next_turn() {
        parts.push(("applies next turn".into(), dim, 1));
    }
    if let Some(branch) = state.selected_git_branch() {
        let files = state.selected_changed_file_count().unwrap_or(0);
        let text = if files > 0 {
            format!("⎇ {branch} +{files}")
        } else {
            format!("⎇ {branch}")
        };
        parts.push((text, dim, 4));
    }
    if let Some(context) = operational_context(state.selected_activity()) {
        parts.push((format!("{context} context"), dim, 3));
    }
    let (status, color) = turn_state(state);
    parts.push((status, Style::default().fg(color), 0));
    let available = usize::from(width.saturating_sub(2));
    for drop in [4, 3, 2, 1] {
        if parts
            .iter()
            .map(|(text, _, _)| text.chars().count() + 3)
            .sum::<usize>()
            <= available
        {
            break;
        }
        parts.retain(|(_, _, priority)| *priority != drop);
    }
    let mut spans = vec![Span::raw(" ")];
    for (index, (text, style, _)) in parts.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", dim));
        }
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

pub(crate) fn turn_state(state: &UiState) -> (String, Color) {
    if state.selected_pending_permission().is_some() {
        return ("needs you".into(), Color::Yellow);
    }
    if state.turn_active() {
        return ("working".into(), Color::Cyan);
    }
    let failed = state.selected_timeline().and_then(|timeline| {
        timeline
            .runs
            .last()
            .and_then(|run| run.turns.iter().max_by_key(|turn| turn.sequence))
            .filter(|turn| turn.phase == DurableTurnPhase::Failed)
    });
    match failed {
        Some(turn)
            if state.selected_transcript().iter().any(|event| {
                event.turn_id == turn.turn_id
                    && event.failure_code == Some(AgentChatFailureCode::ProviderAuthRequired)
            }) =>
        {
            ("sign in needed".into(), Color::Yellow)
        }
        Some(_) => ("last turn failed".into(), Color::Red),
        None => ("ready".into(), Color::Green),
    }
}

pub(crate) fn sign_in_hint(state: &UiState) -> Option<String> {
    let provider = state
        .view
        .as_ref()
        .and_then(|view| view.selection())
        .map_or(state.selection().provider, |selection| selection.provider);
    let name = match provider {
        AgentChatProvider::Claude => "claude",
        AgentChatProvider::Codex => "codex",
        AgentChatProvider::Claurst => return None,
    };
    Some(format!("Type /login {name} to sign in again."))
}
