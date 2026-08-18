//! Prompt-first terminal flow over the same typed local IPC as every other chat client.

use std::path::{Path, PathBuf};

use gent_protocol::OrchestrationFrame;
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
    Orchestration {
        result: Box<OrchestrationFrame>,
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
    if let Some(command) = orchestration_shorthand(&text)? {
        if args.conversation_id.is_some() || args.run_id.is_some() {
            return Err(
                "`/fanout` and `/cross-review` do not accept conversation or run bindings; no worker was started"
                    .into(),
            );
        }
        let result = match command {
            OrchestrationShorthand::Fanout(path) => {
                crate::orchestration_cli::fanout_file(data_dir, no_autostart, path).await?
            }
            OrchestrationShorthand::CrossReview(path) => {
                crate::orchestration_cli::cross_review_file(data_dir, no_autostart, path).await?
            }
        };
        return Ok(Some(DirectPromptResult::Orchestration {
            result: Box::new(result),
        }));
    }
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

enum OrchestrationShorthand {
    Fanout(PathBuf),
    CrossReview(PathBuf),
}

fn orchestration_shorthand(
    text: &str,
) -> Result<Option<OrchestrationShorthand>, Box<dyn std::error::Error>> {
    if let Some(path) = shorthand_path(text, "/fanout")? {
        return Ok(Some(OrchestrationShorthand::Fanout(path)));
    }
    shorthand_path(text, "/cross-review").map(|path| path.map(OrchestrationShorthand::CrossReview))
}

fn shorthand_path(
    text: &str,
    command: &str,
) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    let Some(rest) = text.strip_prefix(command) else {
        return Ok(None);
    };
    if !rest.is_empty() && !rest.chars().next().is_some_and(char::is_whitespace) {
        return Ok(None);
    }
    let paths = rest.split_whitespace().collect::<Vec<_>>();
    match paths.as_slice() {
        [path] => Ok(Some(Path::new(path).to_path_buf())),
        _ => Err(format!(
            "`{command} <path>` requires exactly one JSON file path; no worker was started"
        )
        .into()),
    }
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
    use gent_protocol::{
        Hello, Negotiated, ORCHESTRATION_CAPABILITY, WireFrame, read_frame, read_json_frame,
        write_frame,
    };
    use gent_types::{
        AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRunId, AgentChatSelection,
        CapabilitySet, FanoutRequest, HarnessProfileRef, HostEpoch, PROTOCOL_MAX, TaskGraph,
        TaskGraphBinding, TaskNode, TaskNodeSpec, TaskNodeStatus, TaskRole, WorktreePolicy,
    };
    use tokio::net::UnixListener;

    use super::{DirectPromptArgs, execute, goal_summary, orchestration_shorthand};

    use crate::chat_cli::{Effort, Mode, Provider};

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

    #[test]
    fn slash_orchestration_requires_one_exact_path() {
        assert!(matches!(
            orchestration_shorthand("/fanout graph.json").unwrap(),
            Some(super::OrchestrationShorthand::Fanout(_))
        ));
        assert!(orchestration_shorthand("/cross-review").is_err());
        assert!(orchestration_shorthand("/fanout one.json two.json").is_err());
        assert!(
            orchestration_shorthand("/fanoutting graph.json")
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn slash_fanout_observer_refusal_sends_no_orchestration_frame() {
        let directory = tempfile::tempdir().unwrap();
        let request_path = directory.path().join("graph.json");
        std::fs::write(
            &request_path,
            serde_json::to_string(&fanout_request()).unwrap(),
        )
        .unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                WireFrame::Hello(Hello { capabilities, .. })
                    if capabilities.0.iter().any(|item| item == ORCHESTRATION_CAPABILITY)
            ));
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet::default(),
                }),
            )
            .await
            .unwrap();
            assert!(
                read_json_frame::<_, serde_json::Value>(&mut stream)
                    .await
                    .is_err()
            );
        });
        let error = execute(
            Some(directory.path().into()),
            true,
            DirectPromptArgs {
                prompt: Some(format!("/fanout {}", request_path.display())),
                conversation_id: None,
                run_id: None,
                provider: Provider::Codex,
                model: "unused".into(),
                effort: Effort::Medium,
                mode: Mode::Agent,
            },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("observer mode"));
        server.await.unwrap();
    }

    fn fanout_request() -> FanoutRequest {
        FanoutRequest {
            expected_parent_run_id: AgentChatRunId("run-1".into()),
            graph: TaskGraph {
                binding: TaskGraphBinding {
                    graph_id: "graph-1".into(),
                    conversation_id: gent_types::AgentChatConversationId("conversation-1".into()),
                    root_run_id: AgentChatRunId("run-1".into()),
                    goal_id: "goal-1".into(),
                    goal_revision: 1,
                    policy_id: "policy-1".into(),
                    policy_revision: 1,
                    workspace_id: "workspace-1".into(),
                    repository_id: "repository-1".into(),
                    base_revision_digest_sha256: "a".repeat(64),
                },
                revision: 1,
                host_epoch: HostEpoch(1),
                idempotency_key: "fanout-1".into(),
                nodes: vec![TaskNode {
                    spec: TaskNodeSpec {
                        node_id: "node-1".into(),
                        role: TaskRole::Planner,
                        profile: HarnessProfileRef {
                            profile_id: "profile-1".into(),
                            revision: 1,
                            provider: AgentChatProvider::Codex,
                        },
                        selection: AgentChatSelection {
                            provider: AgentChatProvider::Codex,
                            model: "unused".into(),
                            effort: AgentChatEffort::Medium,
                            mode: AgentChatMode::Agent,
                        },
                        input_artifact_digests: vec![],
                        depends_on: vec![],
                        worktree: WorktreePolicy::Isolated,
                        retry_budget: 0,
                    },
                    revision: 1,
                    status: TaskNodeStatus::Pending,
                    result_artifact_digest: None,
                }],
            },
        }
    }
}
