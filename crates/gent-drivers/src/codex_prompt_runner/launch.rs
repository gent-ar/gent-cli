use gent_ports::PublicProviderRunError;
use gent_types::{FrozenConversationContext, RunVersionLock};

use super::{CodexPromptRunner, lock, map_error};
use crate::codex_runner::CodexRunStart;
use crate::codex_session::CodexSessionConfig;
use crate::conversation_context_input::{
    MAX_FRESH_CONTEXT_INPUT_BYTES, render_fresh_conversation_input,
};
use crate::supervisor::{ProcessLauncher, ProviderProcess};

pub fn fresh_prompt(
    context: Option<&FrozenConversationContext>,
    prompt: &str,
) -> Result<String, PublicProviderRunError> {
    context.map_or_else(
        || Ok(prompt.to_owned()),
        |context| {
            render_fresh_conversation_input(context, prompt, MAX_FRESH_CONTEXT_INPUT_BYTES)
                .map(|input| input.prompt().to_owned())
                .map_err(|_| {
                    PublicProviderRunError::Failed("Codex frozen context is invalid".into())
                })
        },
    )
}

impl<L, P> CodexPromptRunner<L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    pub fn prepare_prewarm(
        &self,
        run_id: String,
        conversation_id: String,
        start: super::CodexPromptStart,
    ) -> Result<(), PublicProviderRunError> {
        self.prepare_for_conversation(run_id.clone(), conversation_id, start)?;
        lock(&self.prewarmed).insert(run_id);
        Ok(())
    }

    pub(super) fn launch(
        &self,
        run_id: &str,
        lock_value: &RunVersionLock,
        resume_thread_id: Option<String>,
    ) -> Result<(), PublicProviderRunError> {
        let prompt = lock(&self.pending).remove(run_id).ok_or_else(|| {
            PublicProviderRunError::Failed("Codex run has no durable pending prompt".into())
        })?;
        let prewarm = lock(&self.prewarmed).remove(run_id);
        if prompt.fresh_context.is_some() && resume_thread_id.is_some() {
            return Err(PublicProviderRunError::Failed(
                "fresh Codex context cannot resume a native session".into(),
            ));
        }
        let rendered_prompt = if prewarm {
            String::new()
        } else {
            fresh_prompt(prompt.fresh_context.as_ref(), &prompt.prompt)?
        };
        if let Some((_, digest)) = self.current_mcp_servers()? {
            lock(&self.mcp_digests).insert(run_id.into(), digest);
        }
        let conversation = lock(&self.pending_mcp_conversations).remove(run_id);
        let selected = prompt.mcp_servers.clone();
        lock(&self.launched_mcp).remove(run_id);
        let start = CodexRunStart {
            run_id: run_id.into(),
            lock: lock_value.clone(),
            session: CodexSessionConfig {
                working_directory: prompt.working_directory,
                resume_thread_id,
                turn_options: prompt.turn_options,
                mcp_servers: prompt.mcp_servers,
            },
            workspace_root: prompt.workspace_root,
            workspace_access: prompt.workspace_access,
            prompt: rendered_prompt,
            goal: prompt.goal,
            attachments: prompt.attachments,
            interrupted_reply: prompt.interrupted_reply,
        };
        let mut runner = lock(&self.runner);
        let result = if prewarm {
            runner.prewarm(start)
        } else {
            runner.start(start)
        }
        .map_err(map_error);
        drop(runner);
        if result.is_ok() {
            if let (Some(selected), Some(conversation)) = (selected, conversation) {
                lock(&self.launched_mcp).insert(run_id.into(), (conversation, selected));
            }
        }
        result
    }
}
