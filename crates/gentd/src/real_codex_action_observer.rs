use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, ToolPhase};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Default)]
pub(super) struct Actions {
    commands: HashSet<String>,
    completed: HashSet<String>,
    output: HashMap<String, String>,
    pub(super) edit_completed: bool,
    pub(super) failed_tools: usize,
    pub(super) diagnostics: BTreeSet<&'static str>,
}

pub(super) fn terminal_diagnostic(effect: &CodexRunnerEffect) -> Option<serde_json::Value> {
    use gent_types::TurnPhase;
    use serde_json::json;
    match effect {
        CodexRunnerEffect::Exited { code } => Some(json!({"kind":"process-exit","code":code})),
        CodexRunnerEffect::ControlRequest(request) => {
            let method = match request.method.as_str() {
                "item/commandExecution/requestApproval" => "command-approval",
                "item/fileChange/requestApproval" => "file-approval",
                "item/permissions/requestApproval" => "permissions-approval",
                "item/tool/requestUserInput" => "user-input",
                "mcpServer/elicitation/request" => "elicitation",
                "item/tool/call" => "dynamic-tool",
                "applyPatchApproval" => "patch-approval",
                "execCommandApproval" => "exec-approval",
                _ => "other",
            };
            Some(json!({"kind":"control-request","method":method}))
        }
        CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase { phase },
        )) => {
            let name = match phase {
                TurnPhase::Failed => "failed",
                TurnPhase::Dead => "dead",
                TurnPhase::Cancelled => "cancelled",
                TurnPhase::Interrupted => "interrupted",
                _ => return None,
            };
            Some(json!({"kind":"root-phase","phase":name}))
        }
        CodexRunnerEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::ProviderFailure {
                classification,
                message,
            },
        )) => Some(
            json!({"kind":"provider-failure","classification":classification,
                          "messageBytes":message.len()}),
        ),
        _ => None,
    }
}

impl Actions {
    pub(super) fn observe(&mut self, fact: &PublicWireFact) {
        match fact {
            PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) => {
                if activity.tool_name == "commandExecution" {
                    self.commands.insert(activity.tool_use_id.clone());
                }
                if activity.phase == ToolPhase::Completed {
                    self.completed.insert(activity.tool_use_id.clone());
                    self.edit_completed |= activity.tool_name == "fileChange";
                }
                self.failed_tools += usize::from(activity.phase == ToolPhase::Failed);
            }
            PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta {
                tool_use_id,
                text,
                ..
            }) => {
                if self.output.len() < 32 || self.output.contains_key(tool_use_id) {
                    let output = self.output.entry(tool_use_id.clone()).or_default();
                    if output.len() + text.len() <= 65536 {
                        output.push_str(text);
                    }
                }
                let lower = text.to_ascii_lowercase();
                for (fragment, label) in [
                    ("powershell", "powershell"),
                    ("bash", "bash"),
                    ("access is denied", "access-denied"),
                    ("os error 5", "os-error-5"),
                    ("not found", "not-found"),
                    ("os error 2", "os-error-2"),
                    ("sandbox", "sandbox"),
                ] {
                    if lower.contains(fragment) {
                        self.diagnostics.insert(label);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn command_printed(&self, marker: &str) -> bool {
        self.commands.iter().any(|id| {
            self.completed.contains(id)
                && self
                    .output
                    .get(id)
                    .is_some_and(|text| text.trim() == marker)
        })
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use gent_drivers::codex_control::CodexControlRequest;
    use gent_types::{ProviderFailureClassification, TurnPhase};
    use serde_json::json;

    #[test]
    fn terminal_kinds_and_exit_status_remain_distinct() {
        assert_eq!(
            terminal_diagnostic(&CodexRunnerEffect::Exited { code: Some(125) }),
            Some(json!({"kind":"process-exit","code":125}))
        );
        assert_eq!(
            terminal_diagnostic(&CodexRunnerEffect::Exited { code: None }),
            Some(json!({"kind":"process-exit","code":null}))
        );
        for (phase, name) in [
            (TurnPhase::Failed, "failed"),
            (TurnPhase::Dead, "dead"),
            (TurnPhase::Cancelled, "cancelled"),
            (TurnPhase::Interrupted, "interrupted"),
        ] {
            assert_eq!(
                terminal_diagnostic(&CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
                    NormalizedLifecycleSignal::RootPhase { phase }
                ))),
                Some(json!({"kind":"root-phase","phase":name}))
            );
        }
        assert!(
            terminal_diagnostic(&CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::RootPhase {
                    phase: TurnPhase::Ready
                }
            )))
            .is_none()
        );
    }

    #[test]
    fn controls_and_failure_messages_never_copy_arbitrary_provider_text() {
        let secret = "secret-qa-canary";
        let request = |method: &str| {
            CodexRunnerEffect::ControlRequest(CodexControlRequest {
                request_id: json!(secret),
                request_key: secret.into(),
                method: method.into(),
                tool_use_id: secret.into(),
                tool_name: secret.into(),
                input: Some(json!({"token":secret})),
            })
        };
        assert_eq!(
            terminal_diagnostic(&request("item/permissions/requestApproval")),
            Some(json!({"kind":"control-request","method":"permissions-approval"}))
        );
        let unknown = terminal_diagnostic(&request(secret)).unwrap().to_string();
        assert!(!unknown.contains(secret));
        let failure = CodexRunnerEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::ProviderFailure {
                classification: ProviderFailureClassification::Provider,
                message: secret.into(),
            },
        ));
        let diagnosis = terminal_diagnostic(&failure).unwrap();
        assert_eq!(diagnosis["kind"], "provider-failure");
        assert_eq!(diagnosis["messageBytes"], secret.len());
        assert!(!diagnosis.to_string().contains(secret));
    }
}
