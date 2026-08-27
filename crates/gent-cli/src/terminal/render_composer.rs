use gent_protocol::LocalModelInstallState;
use ratatui::{
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::state::UiState;

pub(super) fn composer_widget(state: &UiState) -> Paragraph<'static> {
    let color = if state.chat_enabled() {
        Color::Green
    } else {
        Color::Yellow
    };
    let selection = if state.chat_enabled() {
        format!(
            "{} · {} · {:?} · {:?}",
            provider_name(state.selection().provider),
            state.selection().model,
            state.selection().effort,
            state.selection().mode,
        )
    } else {
        "Observer mode: prompts are unavailable.".into()
    };
    let controls = format!(
        "Permissions {} · MCP {} · Context {:?} · Files {}",
        permission_label(state.permission_mode()),
        state.selected_mcp_server_count(),
        state.context_policy(),
        state.attachment_count(),
    );
    let detail = state.picker_line().unwrap_or_else(|| {
        (!state.attachments.is_empty())
            .then(|| attachment_names(state))
            .unwrap_or_else(choices)
    });
    let mut lines = vec![
        Line::from(format!("> {}", state.input())),
        Line::styled(selection, Style::default().fg(color)),
        Line::styled(controls, Style::default().fg(color)),
        Line::styled(detail, Style::default().fg(Color::Cyan)),
    ];
    if let Some(model) = state.selected_local_model_state() {
        lines.push(Line::styled(
            local_model_status(model, &state.selection().model),
            Style::default().fg(Color::DarkGray),
        ));
    }
    if let Some(notice) = state.notice() {
        lines.push(Line::styled(
            notice.to_owned(),
            Style::default().fg(Color::DarkGray),
        ));
    }
    Paragraph::new(lines)
        .wrap(Wrap { trim: true })
        .block(Block::default().borders(Borders::ALL).title("Message"))
}

fn choices() -> String {
    "Tab provider · ^L model · ^E effort · ^O mode · ^P permissions · F1 help".into()
}

fn attachment_names(state: &UiState) -> String {
    let names = state
        .attachments
        .iter()
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
        .take(4)
        .collect::<Vec<_>>();
    let suffix = (state.attachments.len() > names.len())
        .then_some(" …")
        .unwrap_or("");
    format!("Attached · {}{suffix}", names.join(" · "))
}

fn local_model_status(state: &LocalModelInstallState, model: &str) -> String {
    match state {
        LocalModelInstallState::NotInstalled => format!("{model} will download when you send."),
        LocalModelInstallState::Downloading {
            downloaded_bytes,
            total_bytes,
        } => {
            let percent = (*total_bytes > 0)
                .then(|| u128::from(*downloaded_bytes) * 100 / u128::from(*total_bytes));
            percent.map_or_else(
                || {
                    format!(
                        "Downloading {model} model - {downloaded_bytes} bytes - Cancel [Ctrl+C]"
                    )
                },
                |value| format!("Downloading {model} model - {value}% - Cancel [Ctrl+C]"),
            )
        }
        LocalModelInstallState::Ready { .. } => format!("{model} is ready locally."),
    }
}

fn provider_name(provider: gent_types::AgentChatProvider) -> &'static str {
    match provider {
        gent_types::AgentChatProvider::Claude => "Claude",
        gent_types::AgentChatProvider::Codex => "Codex",
        gent_types::AgentChatProvider::Claurst => "Gent (Claurst)",
    }
}

fn permission_label(mode: gent_types::PermissionMode) -> &'static str {
    match mode {
        gent_types::PermissionMode::Default => "ask",
        gent_types::PermissionMode::Plan => "read-only",
        gent_types::PermissionMode::AutoAcceptEdits => "auto edits",
        gent_types::PermissionMode::Autonomous => "autonomous",
        gent_types::PermissionMode::Bypass => "bypass",
    }
}

#[cfg(test)]
mod tests {
    use gent_protocol::LocalModelInstallState;

    use super::local_model_status;

    #[test]
    fn unknown_download_size_never_divides_by_zero() {
        assert_eq!(
            local_model_status(
                &LocalModelInstallState::Downloading {
                    downloaded_bytes: 12,
                    total_bytes: 0,
                },
                "qwen3",
            ),
            "Downloading qwen3 model - 12 bytes - Cancel [Ctrl+C]"
        );
    }
}
