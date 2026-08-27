use super::state::UiState;
use ratatui::{
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};

pub(super) fn picker_widget(state: &UiState) -> Paragraph<'static> {
    if state.documents_visible {
        return document_picker(state);
    }
    if state.automations_visible {
        return automation_picker(state);
    }
    template_picker(state)
}

fn document_picker(state: &UiState) -> Paragraph<'static> {
    let lines = state
        .documents
        .iter()
        .enumerate()
        .map(|(index, document)| {
            picker_line(index == state.document_cursor, &document.relative_path)
        })
        .collect::<Vec<_>>();
    picker("Documents", "↑↓ select · Enter attach · Esc close", lines)
}

fn template_picker(state: &UiState) -> Paragraph<'static> {
    let lines = state
        .templates
        .iter()
        .enumerate()
        .map(|(index, template)| picker_line(index == state.template_cursor, &template.name))
        .collect::<Vec<_>>();
    picker(
        "Prompt templates",
        "↑↓ select · Enter use · Esc close",
        lines,
    )
}

fn automation_picker(state: &UiState) -> Paragraph<'static> {
    let lines = state
        .selected_automations()
        .iter()
        .enumerate()
        .map(|(index, automation)| {
            picker_line(
                index == state.automation_cursor,
                &format!(
                    "{}{}",
                    automation.name,
                    if automation.enabled {
                        ""
                    } else {
                        " · disabled"
                    }
                ),
            )
        })
        .collect::<Vec<_>>();
    picker(
        "Gent automations",
        "↑↓ select · Enter run · Esc close",
        lines,
    )
}

fn picker(
    title: &'static str,
    hint: &'static str,
    mut lines: Vec<Line<'static>>,
) -> Paragraph<'static> {
    if lines.is_empty() {
        lines.push(Line::styled(
            "Nothing is available.",
            Style::default().fg(Color::DarkGray),
        ));
    }
    lines.push(Line::default());
    lines.push(Line::styled(hint, Style::default().fg(Color::Gray)));
    Paragraph::new(lines)
        .wrap(Wrap { trim: true })
        .block(Block::default().borders(Borders::ALL).title(title))
}

fn picker_line(selected: bool, value: &str) -> Line<'static> {
    Line::styled(
        format!("{} {value}", if selected { "›" } else { " " }),
        Style::default()
            .fg(if selected { Color::Cyan } else { Color::Gray })
            .add_modifier(if selected {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
    )
}
