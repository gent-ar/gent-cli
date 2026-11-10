use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::{
    commands::{palette, usage},
    render_help::lines as help_lines,
    state::UiState,
};

const WINDOW: usize = 8;

pub(crate) fn popup_lines(state: &UiState) -> Option<Vec<Line<'static>>> {
    if state.inline().bypass_confirm {
        return Some(vec![
            Line::styled(
                "⚠ Bypass runs without the sandbox and never asks before commands or edits.",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            hint(
                "Enter confirms bypass · Shift+Tab skips to ask every time · Esc keeps the current mode",
            ),
        ]);
    }
    if state.help_visible() {
        let lines = help_lines(state);
        let top = usize::from(state.inline().help_scroll).min(lines.len().saturating_sub(1));
        let mut shown = lines.into_iter().skip(top).take(10).collect::<Vec<_>>();
        shown.push(hint("↑↓ scroll · F1 or Esc closes"));
        return Some(shown);
    }
    if let Some(cursor) = state.conversation_picker() {
        return Some(resume_lines(state, cursor));
    }
    if let Some((title, values, selected)) = state.picker_view() {
        let rows = values
            .iter()
            .map(|value| (value.clone(), String::new()))
            .collect::<Vec<_>>();
        return Some(list(
            &title,
            &rows,
            selected,
            "↑↓ choose · Enter apply · Esc cancel",
        ));
    }
    let catalog = state.command_catalog()?;
    let matches = palette(catalog, state.input());
    if matches.is_empty() {
        return None;
    }
    let rows = matches
        .iter()
        .map(|command| (usage(command), String::new()))
        .collect::<Vec<_>>();
    Some(list(
        "Commands",
        &rows,
        state.inline().palette_cursor.min(rows.len() - 1),
        "↑↓ choose · Enter runs · Esc clears",
    ))
}

fn resume_lines(state: &UiState, cursor: usize) -> Vec<Line<'static>> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| {
            u64::try_from(value.as_millis()).unwrap_or(u64::MAX)
        });
    let rows = state
        .conversations()
        .iter()
        .map(|item| {
            let metadata = state.metadata(&item.conversation_id);
            let title = metadata
                .and_then(|value| value.title.clone())
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| "Untitled conversation".into());
            let workspace = metadata
                .and_then(|value| value.workspace_path.as_deref())
                .map(|path| {
                    path.trim_end_matches(['\\', '/'])
                        .rsplit(['\\', '/'])
                        .next()
                        .unwrap_or(path)
                        .to_owned()
                })
                .unwrap_or_default();
            let age = metadata.map_or(String::new(), |value| age(now, value.updated_at_unix_ms));
            let running = if metadata.is_some_and(|value| value.running) {
                "● running  "
            } else {
                ""
            };
            (title, format!("{running}{workspace} · {age}"))
        })
        .collect::<Vec<_>>();
    list(
        "Resume a conversation",
        &rows,
        cursor,
        "↑↓ select · Enter open · Esc cancel",
    )
}

fn age(now: u64, then: u64) -> String {
    if then == 0 {
        return String::new();
    }
    let minutes = now.saturating_sub(then) / 60_000;
    match minutes {
        0 => "just now".into(),
        1..=59 => format!("{minutes} min ago"),
        60..=1439 => format!("{} h ago", minutes / 60),
        _ => format!("{} d ago", minutes / 1440),
    }
}

fn list(title: &str, rows: &[(String, String)], cursor: usize, footer: &str) -> Vec<Line<'static>> {
    let start = cursor
        .saturating_sub(WINDOW - 1)
        .min(rows.len().saturating_sub(WINDOW));
    let mut lines = vec![Line::styled(
        format!("{title}{}", counter(cursor, rows.len())),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    for (index, (label, detail)) in rows.iter().enumerate().skip(start).take(WINDOW) {
        let selected = index == cursor;
        let style = if selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let mut spans = vec![Span::styled(
            format!("{} {label}", if selected { "❯" } else { " " }),
            style,
        )];
        if !detail.is_empty() {
            spans.push(Span::styled(
                format!("  {detail}"),
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(hint(footer));
    lines
}

fn counter(cursor: usize, count: usize) -> String {
    if count > WINDOW {
        format!("  {}/{count}", cursor + 1)
    } else {
        String::new()
    }
}

fn hint(text: &str) -> Line<'static> {
    Line::styled(text.to_owned(), Style::default().fg(Color::DarkGray))
}
