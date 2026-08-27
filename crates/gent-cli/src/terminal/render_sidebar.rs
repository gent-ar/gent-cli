use ratatui::{
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::state::UiState;

pub(super) fn workspace_widget(state: &UiState, width: u16, height: u16) -> Paragraph<'static> {
    let compact = width < 34;
    let workspace = state.selected_workspace_path().map_or_else(
        || "No workspace".to_owned(),
        |path| clip_path(path, usize::from(width.saturating_sub(4))),
    );
    let mut lines = vec![Line::styled(
        workspace,
        Style::default().add_modifier(Modifier::BOLD),
    )];
    lines.push(fact(git_label(state, compact)));
    let mcp = state.selected_mcp_server_names();
    lines.push(fact(mcp_label(
        mcp,
        state.selected_mcp_server_count(),
        compact,
    )));
    lines.push(section(
        "Activity",
        state.selected_activity().len(),
        "F2",
        compact,
    ));
    lines.push(section(
        "Automations",
        usize::from(state.selected_automation_count()),
        "/automations",
        compact,
    ));
    lines.push(section(
        "Docs",
        state.documents.len(),
        "/documents",
        compact,
    ));
    lines.push(section(
        "Templates",
        state.templates.len(),
        "/templates",
        compact,
    ));
    if height > 10 && !compact {
        detail_lines(state, &mut lines);
    }
    lines.truncate(usize::from(height.saturating_sub(2)));
    Paragraph::new(lines)
        .wrap(Wrap { trim: true })
        .block(Block::default().borders(Borders::ALL).title("Workspace"))
}

fn detail_lines(state: &UiState, lines: &mut Vec<Line<'static>>) {
    lines.push(fact(catalog_label(
        "Automation catalog",
        state.selected_automation_names(),
        state.selected_automation_count(),
    )));
    lines.push(fact(automation_run_label(state.selected_automation_runs())));
    for document in state.documents.iter().take(2) {
        lines.push(fact(format!(
            "{} · {}",
            document.relative_path,
            &document.document_id[..12]
        )));
    }
    if state.documents.len() > 2 {
        lines.push(fact(format!(
            "  +{} more · /documents",
            state.documents.len() - 2
        )));
    }
    for (index, template) in state.templates.iter().take(2).enumerate() {
        lines.push(fact(format!(
            "{}{} · {}",
            if state.templates_visible && index == state.template_cursor {
                "> "
            } else {
                "  "
            },
            template.name,
            template.template_id
        )));
    }
    if state.templates.len() > 2 {
        lines.push(fact(format!(
            "  +{} more · /templates",
            state.templates.len() - 2
        )));
    }
    lines.push(fact(catalog_label(
        "Forge",
        state.selected_forge_names(),
        state.selected_forge_count(),
    )));
    let activity = activity_label(state.selected_activity());
    lines.push(Line::styled(activity, Style::default().fg(Color::Cyan)));
}

fn git_label(state: &UiState, compact: bool) -> String {
    match (
        state.selected_git_branch(),
        state.selected_changed_file_count(),
    ) {
        (Some(branch), Some(files)) if compact => format!("Git · {branch} · {files}"),
        (Some(branch), Some(files)) => format!("Git · {branch} · {files} changed"),
        (Some(branch), None) => format!("Git · {branch}"),
        (None, Some(files)) => format!("Files · {files} changed"),
        (None, None) => "Git · unavailable".into(),
    }
}

fn clip_path(path: &str, limit: usize) -> String {
    let limit = limit.max(1);
    let value = path.chars().collect::<Vec<_>>();
    if value.len() <= limit {
        return path.to_owned();
    }
    let suffix = value[value.len() - limit.saturating_sub(1)..]
        .iter()
        .collect::<String>();
    format!("…{suffix}")
}

fn mcp_label(names: &[String], count: u16, compact: bool) -> String {
    if names.is_empty() || compact {
        return format!("MCP · {count}");
    }
    let visible = names.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
    let more = names.len().saturating_sub(2);
    format!(
        "MCP · {visible}{}",
        (more > 0).then(|| format!(" +{more}")).unwrap_or_default()
    )
}

fn catalog_label(label: &str, names: &[String], count: u16) -> String {
    if names.is_empty() {
        return format!("{label} · {count}");
    }
    let visible = names.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
    let more = names.len().saturating_sub(2);
    format!(
        "{label} · {visible}{}",
        (more > 0).then(|| format!(" +{more}")).unwrap_or_default()
    )
}

fn activity_label(facts: &[gent_types::ConversationActivityFact]) -> String {
    let (tools, subagents, processes) = super::render_activity::counts(facts);
    format!("Live · {tools} tools · {processes} processes · {subagents} subagents")
}

fn automation_run_label(runs: &[gent_types::AutomationRunSummary]) -> String {
    let Some(run) = runs.first() else {
        return "Automation runs · none".into();
    };
    format!("Automation runs · latest {:?}", run.status)
}

fn fact(value: String) -> Line<'static> {
    Line::styled(value, Style::default().fg(Color::Gray))
}

fn section(label: &str, count: usize, command: &str, compact: bool) -> Line<'static> {
    Line::styled(
        if compact {
            format!("{label} · {count}")
        } else {
            format!("{label} · {count} · {command}")
        },
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
}
