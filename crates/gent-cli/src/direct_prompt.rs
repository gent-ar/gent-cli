//! Prompt-first terminal flow over the same typed local IPC as every other chat client.

use std::path::PathBuf;

use gent_types::{AgentChatConversationId, AgentChatSelection, GoalRecord};
use serde::Serialize;

use crate::{
    chat_cli::{self, DirectPromptArgs, effort, mode, provider},
    goal_cli,
};

/// Public terminal result after durable prompt submission; it never claims provider execution.
#[derive(Debug, Serialize)]
#[serde(untagged, rename_all = "camelCase")]
pub(crate) enum DirectPromptResult {
    Prompt {
        conversation_id: String,
        run_id: Option<String>,
        delivery: gent_types::AgentChatPromptDelivery,
    },
    Goal {
        goal: GoalRecord,
    },
}

/// Creates a selected conversation when needed, then durably submits one terminal prompt.
pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: DirectPromptArgs,
) -> Result<Option<DirectPromptResult>, Box<dyn std::error::Error>> {
    let Some(text) = args.prompt else {
        if args.run_id.is_some() {
            return Err("--run-id requires a positional prompt".into());
        }
        return Ok(None);
    };
    if let Some(summary) = goal_summary(&text)? {
        let (Some(conversation_id), Some(run_id)) = (args.conversation_id, args.run_id) else {
            return Err(
                "`/goal <summary>` requires --conversation-id and --run-id; no provider work was started"
                    .into(),
            );
        };
        return goal_cli::create_shorthand(
            data_dir,
            no_autostart,
            conversation_id,
            run_id,
            summary,
        )
        .await
        .map(|goal| Some(DirectPromptResult::Goal { goal }));
    }
    if args.run_id.is_some() {
        return Err("--run-id is only valid with positional `/goal <summary>`".into());
    }
    let (conversation_id, run_id) = if let Some(conversation_id) = args.conversation_id {
        (AgentChatConversationId(conversation_id), None)
    } else {
        let selection = AgentChatSelection {
            provider: provider(args.provider),
            model: args.model,
            effort: effort(args.effort),
            mode: mode(args.mode),
        };
        let (conversation_id, run_id) =
            chat_cli::create(data_dir.clone(), no_autostart, selection).await?;
        (conversation_id, Some(run_id))
    };
    let delivery = chat_cli::send(data_dir, no_autostart, conversation_id.0.clone(), text).await?;
    Ok(Some(DirectPromptResult::Prompt {
        conversation_id: conversation_id.0,
        run_id: run_id.map(|value| value.0),
        delivery,
    }))
}

fn goal_summary(text: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let Some(summary) = text.strip_prefix("/goal") else {
        return Ok(None);
    };
    if summary.is_empty() {
        return Err("`/goal` requires a concise summary; no provider work was started".into());
    }
    if !summary.chars().next().is_some_and(char::is_whitespace) {
        return Ok(None);
    }
    let summary = summary.trim();
    if summary.is_empty() {
        return Err("`/goal` requires a concise summary; no provider work was started".into());
    }
    Ok(Some(summary.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::goal_summary;

    #[test]
    fn only_the_exact_goal_slash_command_is_recognized() {
        assert_eq!(goal_summary("/goals list").unwrap(), None);
        assert_eq!(goal_summary("normal prompt").unwrap(), None);
        assert_eq!(
            goal_summary("/goal ship it").unwrap(),
            Some("ship it".into())
        );
        assert!(goal_summary("/goal").is_err());
    }
}
