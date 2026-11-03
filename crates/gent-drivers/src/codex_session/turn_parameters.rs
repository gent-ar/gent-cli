use super::{CodexSandboxPolicy, CodexTurnOptions};
use serde_json::{Value, json};

pub(crate) fn turn_parameters(
    options: &CodexTurnOptions,
    thread_id: &str,
    prompt: &str,
    attachments: &[Value],
) -> Value {
    let sandbox_policy = match options.sandbox() {
        CodexSandboxPolicy::ReadOnly => json!({"type": "readOnly", "networkAccess": false}),
        CodexSandboxPolicy::WorkspaceWrite { writable_root } => json!({
            "type": "workspaceWrite",
            "writableRoots": writable_root.iter().collect::<Vec<_>>(),
            "networkAccess": false,
            "excludeTmpdirEnvVar": false,
            "excludeSlashTmp": false,
        }),
        CodexSandboxPolicy::DangerFullAccess => json!({"type": "dangerFullAccess"}),
        CodexSandboxPolicy::ExternalSandbox => {
            json!({"type": "externalSandbox", "networkAccess": "enabled"})
        }
    };
    let prompt = options
        .configured_append_system_prompt
        .as_deref()
        .map_or_else(
            || prompt.into(),
            |instruction| format!("{instruction}\n\nUser request:\n{prompt}"),
        );
    let mut parameters = json!({
        "threadId": thread_id,
        "input": [{"type": "text", "text": prompt}],
        "effort": options.effort(),
        "summary": "auto",
        "approvalPolicy": options.approval(),
        "sandboxPolicy": sandbox_policy,
    });
    if let Some(model) = options.model() {
        parameters["model"] = Value::String(model.into());
        if options.plan {
            parameters["collaborationMode"] = json!({
                "mode": "plan",
                "settings": {"model": model, "reasoning_effort": options.effort(), "developer_instructions": null},
            });
        }
    }
    if !attachments.is_empty() {
        let input = parameters["input"]
            .as_array_mut()
            .expect("fixed input array");
        input.extend(attachments.iter().cloned());
    }
    parameters
}
