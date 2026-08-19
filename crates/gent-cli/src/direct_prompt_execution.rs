//! Terminal composition for a direct prompt and optional durable turn follow.

use std::path::PathBuf;

use crate::{
    chat_cli::{DirectPromptArgs, turn_follow},
    command_execution::print,
    direct_prompt::{self, DirectPromptResult},
};

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: DirectPromptArgs,
) -> Result<bool, Box<dyn std::error::Error>> {
    let Some(reply) = direct_prompt::execute(data_dir.clone(), no_autostart, args).await? else {
        return Ok(false);
    };
    print(&reply)?;
    if let DirectPromptResult::Prompt {
        conversation_id,
        run_id,
        turn_id,
        ..
    } = reply
    {
        let _ = turn_follow::follow_accepted_if_supported(
            data_dir,
            no_autostart,
            conversation_id,
            run_id,
            turn_id,
        )
        .await?;
    }
    Ok(true)
}
