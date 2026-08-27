use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::api::RuntimeApi;
use gent_protocol::{AgentChatIntentFrame, AutomationFrame};
use gent_runtime::AutomationResult;
use gent_types::{
    AgentChatRequestId, AutomationAction, AutomationId, AutomationRun, AutomationRunId,
    AutomationRunStatus, ReceiptId,
};

use crate::runtime_facade::RuntimeFacade;

impl RuntimeFacade {
    pub(crate) fn run_automation(
        &self,
        request_id: String,
        automation_id: AutomationId,
    ) -> Result<AutomationFrame, String> {
        let definition = match self
            .automations
            .get(&automation_id)
            .map_err(|error| error.to_string())?
        {
            AutomationResult::Definition(definition) => definition,
            AutomationResult::Missing => return Err("automation does not exist".into()),
            AutomationResult::DeniedObserver => {
                return Err("automations are unavailable while gentd is observer-disabled".into());
            }
            _ => return Err("automation lookup returned an invalid result".into()),
        };
        if let AutomationAction::Script { command, args } = &definition.action {
            return self.run_script(request_id, automation_id, &definition, command, args);
        }
        let text = action_text(&definition.action)?;
        let create_request = format!("automation-create-{request_id}");
        let created = self.agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId(create_request),
            receipt_id: ReceiptId(format!("automation-receipt-create-{request_id}")),
            workspace_path: definition.working_directory.clone(),
            selection: definition.selection.clone(),
        })?;
        let AgentChatIntentFrame::Created {
            conversation_id,
            run_id: created_agent_chat_run_id,
            ..
        } = created
            .into_iter()
            .next()
            .ok_or("automation conversation was not created")?
        else {
            return Err("automation conversation returned an invalid response".into());
        };
        let prompt = self.agent_chat_intent(AgentChatIntentFrame::SendPrompt {
            request_id: AgentChatRequestId(format!("automation-prompt-{request_id}")),
            receipt_id: ReceiptId(format!("automation-receipt-prompt-{request_id}")),
            conversation_id: conversation_id.clone(),
            text,
            attachment_ids: Vec::new(),
        })?;
        let AgentChatIntentFrame::Accepted {
            run_id, turn_id, ..
        } = prompt
            .into_iter()
            .next()
            .ok_or("automation prompt was not accepted")?
        else {
            return Err("automation prompt returned an invalid response".into());
        };
        if run_id != created_agent_chat_run_id {
            return Err("automation prompt selected a different chat run".into());
        }
        let automation_run_id = AutomationRunId(format!("automation-run-{request_id}"));
        self.automations
            .record_run(AutomationRun {
                run_id: automation_run_id.clone(),
                automation_id: automation_id.clone(),
                conversation_id: Some(conversation_id.0.clone()),
                parent_run_id: None,
                started_at: now_millis(),
                ended_at: None,
                status: AutomationRunStatus::Running,
                summary: None,
                error: None,
                condition_result: None,
            })
            .map_err(|error| error.to_string())?;
        Ok(AutomationFrame::RunAccepted {
            request_id,
            automation_id,
            run_id: automation_run_id,
            conversation_id: conversation_id.0,
            agent_chat_run_id: run_id.0,
            turn_id,
        })
    }

    fn run_script(
        &self,
        request_id: String,
        automation_id: AutomationId,
        definition: &gent_types::AutomationDefinition,
        command: &str,
        args: &[String],
    ) -> Result<AutomationFrame, String> {
        let run_id = AutomationRunId(format!("automation-run-{request_id}"));
        let started_at = now_millis();
        let result = execute_script(&definition.working_directory, command, args);
        let ended_at = now_millis();
        let (status, summary, error) = match result {
            Ok(output) => (AutomationRunStatus::Success, Some(output), None),
            Err(error) => (AutomationRunStatus::Error, None, Some(error)),
        };
        self.automations
            .record_run(AutomationRun {
                run_id: run_id.clone(),
                automation_id: automation_id.clone(),
                conversation_id: None,
                parent_run_id: None,
                started_at,
                ended_at: Some(ended_at),
                status,
                summary,
                error,
                condition_result: None,
            })
            .map_err(|error| error.to_string())?;
        Ok(AutomationFrame::RunAccepted {
            request_id,
            automation_id,
            run_id: run_id.clone(),
            conversation_id: format!("automation-{}", run_id.0),
            agent_chat_run_id: format!("script-{}", run_id.0),
            turn_id: format!("script-turn-{}", run_id.0),
        })
    }
}

fn action_text(action: &AutomationAction) -> Result<String, String> {
    match action {
        AutomationAction::Prompt { prompt } => Ok(prompt.clone()),
        AutomationAction::Skill { skill } => Ok(format!("Run the {skill} skill.")),
        AutomationAction::SkillAndPrompt { skill, prompt } => {
            Ok(format!("Run the {skill} skill.\n\n{prompt}"))
        }
        AutomationAction::Script { .. } => Err("script automation execution is unavailable".into()),
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

const SCRIPT_OUTPUT_BYTES: usize = 64 * 1024;
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(30);

fn execute_script(
    working_directory: &str,
    command: &str,
    args: &[String],
) -> Result<String, String> {
    let root = std::fs::canonicalize(working_directory)
        .map_err(|_| "automation workspace is not accessible".to_owned())?;
    let candidate = if Path::new(command).is_absolute() {
        PathBuf::from(command)
    } else {
        root.join(command)
    };
    let executable = std::fs::canonicalize(candidate)
        .map_err(|_| "automation script is not present in its workspace".to_owned())?;
    if !executable.starts_with(&root)
        || !executable.is_file()
        || command.contains('\0')
        || args.iter().any(|arg| arg.contains('\0'))
    {
        return Err("automation script must be a workspace file".into());
    }
    let mut child = Command::new(&executable)
        .args(args)
        .current_dir(&root)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "automation script could not be started".to_owned())?;
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout = read_pipe(
        child.stdout.take().ok_or("automation stdout unavailable")?,
        Arc::clone(&overflow),
    );
    let stderr = read_pipe(
        child.stderr.take().ok_or("automation stderr unavailable")?,
        Arc::clone(&overflow),
    );
    let deadline = std::time::Instant::now() + SCRIPT_TIMEOUT;
    loop {
        if overflow.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("automation script output exceeded 64 KiB".into());
        }
        if child
            .try_wait()
            .map_err(|_| "automation script status failed")?
            .is_some()
        {
            break;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("automation script exceeded its 30 second limit".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    let status = child
        .wait()
        .map_err(|_| "automation script status failed")?;
    let output = format!(
        "{}{}",
        stdout.join().unwrap_or_default(),
        stderr.join().unwrap_or_default()
    );
    if output.len() > SCRIPT_OUTPUT_BYTES {
        return Err("automation script output exceeded 64 KiB".into());
    }
    if status.success() {
        Ok(output)
    } else {
        Err(format!("automation script exited unsuccessfully: {output}"))
    }
}

fn read_pipe<R: Read + Send + 'static>(
    mut pipe: R,
    overflow: Arc<AtomicBool>,
) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0_u8; 8192];
        while let Ok(count) = pipe.read(&mut buffer) {
            if count == 0 {
                break;
            }
            if output.len().saturating_add(count) > SCRIPT_OUTPUT_BYTES {
                overflow.store(true, Ordering::Release);
                break;
            }
            output.extend_from_slice(&buffer[..count]);
        }
        String::from_utf8_lossy(&output).into_owned()
    })
}
