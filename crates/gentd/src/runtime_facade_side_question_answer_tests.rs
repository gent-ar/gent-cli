use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use gent_drivers::conversation_context_summary::SummaryRequest;
use gent_ports::PortError;
use gent_protocol::{
    AgentChatProjectionFrame, AgentChatSideQuestionFrame, ProjectionCursor, read_json_frame,
    write_json_frame,
};
use gent_types::ContextCompactionFailure;
use serde_json::{Value, json};

use super::{facade_with, root, root_with_selection};
use crate::agent_chat_side_question_runners::AgentChatSideQuestionRunnerSources;
use crate::claurst_runtime_factory::{ContextSummarizer, ContextSummary, LocalContextWindow};
use crate::claurst_side_question_runner::ClaurstSideQuestionModels;

#[derive(Debug, Default)]
struct LocalModel {
    asked: Mutex<Vec<(String, String)>>,
}

#[derive(Debug)]
struct Loaded;

#[async_trait]
impl ContextSummarizer for Loaded {
    fn window(&self) -> LocalContextWindow {
        LocalContextWindow {
            history_input_bytes: 1024,
            summary_input_bytes: 1024,
        }
    }

    async fn summarize(
        &self,
        request: SummaryRequest,
    ) -> Result<ContextSummary, ContextCompactionFailure> {
        let answer = if request.content.contains("What word did you reply with?") {
            "You replied PONG."
        } else {
            ""
        };
        Ok(ContextSummary {
            text: answer.into(),
            tokens: 4,
        })
    }
}

#[async_trait]
impl ClaurstSideQuestionModels for Arc<LocalModel> {
    async fn answer_model(
        &self,
        conversation_id: &str,
        model_id: &str,
    ) -> Result<Arc<dyn ContextSummarizer>, PortError> {
        self.asked
            .lock()
            .unwrap()
            .push((conversation_id.into(), model_id.into()));
        Ok(Arc::new(Loaded))
    }
}

#[tokio::test]
async fn an_idle_gent_local_side_question_is_answered_by_the_conversations_local_model() {
    let directory = tempfile::tempdir().unwrap();
    let model = Arc::new(LocalModel::default());
    let runtime = facade_with(
        directory.path(),
        vec![],
        Some(AgentChatSideQuestionRunnerSources {
            data_dir: directory.path().to_path_buf(),
            executables: crate::provider_executables::ProviderExecutables::explicit(None, None),
            claurst_models: Some(Arc::new(Arc::clone(&model))),
        }),
    )
    .await;
    let conversation = root(&runtime, "answer", ".");
    let fact =
        settled_side_question(&runtime, &conversation, "What word did you reply with?").await;
    assert_eq!(
        json!({"status": fact["status"], "answer": fact["answer"], "failureCode": fact["failureCode"]}),
        json!({"status": "answered", "answer": "You replied PONG.", "failureCode": null})
    );
    assert_eq!(
        *model.asked.lock().unwrap(),
        [(conversation.0.clone(), "qwen3-1-7b-q4-k-m".to_owned())]
    );
}

async fn settled_side_question(
    runtime: &crate::runtime_facade::RuntimeFacade,
    conversation: &gent_types::AgentChatConversationId,
    question: &str,
) -> Value {
    let mut follower = super::projection_window_tests::connect(runtime).await;
    write_json_frame(
        &mut follower,
        &AgentChatProjectionFrame::FollowConversation {
            request_id: "follow".into(),
            conversation_id: conversation.0.clone(),
            after_cursor: ProjectionCursor { value: 0 },
        },
    )
    .await
    .unwrap();
    let mut client = super::side_question_busy_tests::connect(runtime).await;
    write_json_frame(
        &mut client,
        &AgentChatSideQuestionFrame::AskSideQuestion {
            request_id: "ask".into(),
            receipt_id: "ask-receipt".into(),
            conversation_id: conversation.0.clone(),
            question: question.into(),
        },
    )
    .await
    .unwrap();
    let asked: Value = read_json_frame(&mut client).await.unwrap();
    assert_eq!(
        (&asked["type"], &asked["body"]["record"]["status"]),
        (&json!("asked"), &json!("pending"))
    );
    loop {
        let delta: Value = read_json_frame(&mut follower).await.unwrap();
        let fact = delta["body"]["delta"]["body"]["fact"].clone();
        if fact["type"] == "sideQuestionSettled" {
            return fact;
        }
    }
}

fn claude_stream(answer: &str) -> String {
    let replayed_excerpt =
        "Assistant: a long earlier answer about ledgers and steers. ".repeat(256);
    let mut frames = vec![
        json!({"type": "system", "subtype": "init", "model": "claude-haiku-4-5", "tools": []}),
        json!({"type": "user", "isReplay": true, "message": {"role": "user", "content": [{"type": "text", "text": replayed_excerpt}]}}),
    ];
    frames.extend(answer.split_inclusive(' ').map(|word| {
        json!({"type": "stream_event", "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": word}}})
    }));
    frames.push(json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": answer}]}}));
    frames
        .push(json!({"type": "result", "subtype": "success", "is_error": false, "result": answer}));
    frames
        .iter()
        .map(|frame| frame.to_string() + "\n")
        .collect()
}

fn fake_claude(directory: &std::path::Path, stream: &str) -> std::path::PathBuf {
    std::fs::write(directory.join("claude-stream.jsonl"), stream).unwrap();
    #[cfg(windows)]
    let (name, script) = (
        "claude.cmd",
        "@powershell -NoProfile -NonInteractive -Command \"$null = [Console]::In.ReadToEnd(); [Console]::Out.Write([IO.File]::ReadAllText('%~dp0claude-stream.jsonl'))\"\r\n",
    );
    #[cfg(unix)]
    let (name, script) = (
        "claude",
        "#!/bin/sh\ncat >/dev/null\ncat \"$(dirname \"$0\")/claude-stream.jsonl\"\n",
    );
    let path = directory.join(name);
    std::fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

#[tokio::test]
async fn a_claude_side_question_delivers_a_long_answer_whose_stream_replays_a_long_conversation() {
    let directory = tempfile::tempdir().unwrap();
    let answer = "Steers land in the running turn while the ledger keeps every fact. ".repeat(600);
    let runtime = facade_with(
        directory.path(),
        vec![],
        Some(claude_sources(directory.path(), &claude_stream(&answer))),
    )
    .await;
    let conversation = root_with_selection(&runtime, "claude-answer", ".", claude_selection());
    let fact = settled_side_question(
        &runtime,
        &conversation,
        "How do steers relate to the ledger?",
    )
    .await;
    assert_eq!(
        json!({"status": fact["status"], "answer": fact["answer"], "failureCode": fact["failureCode"]}),
        json!({"status": "answered", "answer": answer, "failureCode": null})
    );
}

fn claude_sources(directory: &std::path::Path, stream: &str) -> AgentChatSideQuestionRunnerSources {
    AgentChatSideQuestionRunnerSources {
        data_dir: directory.to_path_buf(),
        executables: crate::provider_executables::ProviderExecutables::explicit(
            Some(fake_claude(directory, stream)),
            None,
        ),
        claurst_models: None,
    }
}

#[tokio::test]
async fn a_claude_side_question_that_claude_answers_with_an_error_settles_as_provider_failed() {
    let directory = tempfile::tempdir().unwrap();
    let stream = format!(
        "{}\n",
        json!({"type": "result", "subtype": "success", "is_error": true, "result": "API Error: 500"})
    );
    let runtime = facade_with(
        directory.path(),
        vec![],
        Some(claude_sources(directory.path(), &stream)),
    )
    .await;
    let conversation = root_with_selection(&runtime, "claude-error", ".", claude_selection());
    let fact = settled_side_question(&runtime, &conversation, "Why?").await;
    assert_eq!(
        json!({"status": fact["status"], "answer": fact["answer"], "failureCode": fact["failureCode"]}),
        json!({"status": "failed", "answer": null, "failureCode": "providerFailed"})
    );
}

fn claude_selection() -> gent_types::AgentChatSelection {
    gent_types::AgentChatSelection {
        provider: gent_types::AgentChatProvider::Claude,
        model: "haiku".into(),
        effort: gent_types::AgentChatEffort::Medium,
        mode: gent_types::AgentChatMode::Agent,
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_codex_side_question_delivers_an_answer_longer_than_a_title_bound() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let codex = directory.path().join("cli/codex");
    std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
    std::fs::write(&codex, include_str!("../testdata/fake-codex-app-server.py")).unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
    let runtime = facade_with(
        directory.path(),
        vec![],
        Some(AgentChatSideQuestionRunnerSources {
            data_dir: directory.path().to_path_buf(),
            executables: crate::provider_executables::ProviderExecutables::explicit(
                None,
                Some(codex),
            ),
            claurst_models: None,
        }),
    )
    .await;
    let conversation = root_with_selection(
        &runtime,
        "codex-answer",
        &directory.path().to_string_lossy(),
        gent_types::AgentChatSelection {
            provider: gent_types::AgentChatProvider::Codex,
            model: "gpt-5.4-mini".into(),
            effort: gent_types::AgentChatEffort::Medium,
            mode: gent_types::AgentChatMode::Agent,
        },
    );
    let fact = settled_side_question(&runtime, &conversation, "Give the LONG answer").await;
    assert_eq!(
        json!({"status": fact["status"], "answer": fact["answer"], "failureCode": fact["failureCode"]}),
        json!({"status": "answered", "answer": "lighthouse ".repeat(180 * 48), "failureCode": null})
    );
}
