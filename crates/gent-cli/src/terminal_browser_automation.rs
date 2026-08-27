use std::path::PathBuf;

use crate::{automation_cli, terminal::UiRequestResult};

pub(crate) async fn run(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    automation_id: String,
    current_conversation_id: String,
) -> Result<UiRequestResult, String> {
    let frame = automation_cli::run(
        data_dir.clone(),
        no_autostart,
        gent_types::AutomationId(automation_id.clone()),
    )
    .await
    .map_err(|error| error.to_string())?;
    let gent_protocol::AutomationFrame::RunAccepted {
        conversation_id,
        agent_chat_run_id,
        ..
    } = frame
    else {
        return Err("daemon returned an invalid automation run response".into());
    };
    if agent_chat_run_id.starts_with("script-") {
        let runs = automation_cli::runs(
            data_dir,
            no_autostart,
            gent_types::AutomationId(automation_id),
            1,
        )
        .await
        .map_err(|error| error.to_string())?;
        let notice = runs.first().map_or_else(
            || "Automation script completed.".into(),
            |run| match (&run.status, &run.summary, &run.error) {
                (gent_types::AutomationRunStatus::Success, Some(summary), _) => {
                    format!("Automation script completed: {}", clip(summary))
                }
                (_, _, Some(error)) => format!("Automation script failed: {error}"),
                _ => "Automation script completed.".into(),
            },
        );
        return Ok(UiRequestResult {
            conversation: gent_types::ConversationListItem {
                conversation_id: current_conversation_id,
                run_count: 1,
            },
            parent_run_id: None,
            notice,
            permission_mode: None,
            session: None,
            awaiting_turn: None,
        });
    }
    Ok(UiRequestResult {
        conversation: gent_types::ConversationListItem {
            conversation_id,
            run_count: 1,
        },
        parent_run_id: Some(agent_chat_run_id),
        notice: "Automation run started in a new Gent conversation.".into(),
        permission_mode: None,
        session: None,
        awaiting_turn: None,
    })
}

fn clip(value: &str) -> String {
    const LIMIT: usize = 512;
    let mut result = value.chars().take(LIMIT + 1).collect::<String>();
    if result.chars().count() > LIMIT {
        result.truncate(
            result
                .char_indices()
                .nth(LIMIT)
                .map_or(result.len(), |(index, _)| index),
        );
        result.push('…');
    }
    result
}
