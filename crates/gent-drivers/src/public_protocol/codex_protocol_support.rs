use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent};
use serde_json::Value;

use super::PublicWireFact;

pub(super) fn attention() -> Vec<PublicWireFact> {
    vec![PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::AttentionRequired,
    )]
}
pub(super) fn tool_kind(kind: &str) -> bool {
    matches!(
        kind,
        "commandExecution"
            | "fileChange"
            | "mcpToolCall"
            | "dynamicToolCall"
            | "collabAgentToolCall"
            | "collabToolCall"
            | "webSearch"
            | "imageView"
            | "imageGeneration"
            | "hookPrompt"
    )
}
pub(super) fn inert_item(kind: &str) -> bool {
    matches!(
        kind,
        "agentMessage"
            | "userMessage"
            | "reasoning"
            | "plan"
            | "enteredReviewMode"
            | "exitedReviewMode"
            | "contextCompaction"
    )
}
pub(super) fn housekeeping(method: &str) -> bool {
    matches!(
        method,
        "thread/settings/updated"
            | "thread/tokenUsage/updated"
            | "thread/name/updated"
            | "turn/diff/updated"
            | "account/rateLimits/updated"
            | "skills/changed"
            | "remoteControl/status/changed"
            | "item/commandExecution/outputDelta"
            | "item/commandExecution/terminalInteraction"
            | "item/fileChange/outputDelta"
            | "item/fileChange/patchUpdated"
            | "item/tool/call"
            | "turn/moderationMetadata"
            | "mcpServer/startupStatus/updated"
            | "warning"
            | "guardianWarning"
            | "deprecationNotice"
            | "configWarning"
            | "model/rerouted"
            | "model/verification"
            | "serverRequest/resolved"
            | "app/list/updated"
            | "command/exec/outputDelta"
            | "externalAgentConfig/import/progress"
            | "externalAgentConfig/import/completed"
            | "fs/changed"
            | "fuzzyFileSearch/sessionUpdated"
            | "fuzzyFileSearch/sessionCompleted"
            | "process/outputDelta"
            | "process/exited"
            | "thread/archived"
            | "thread/unarchived"
            | "thread/deleted"
            | "thread/closed"
            | "thread/goal/updated"
            | "thread/goal/cleared"
            | "account/updated"
            | "model/safetyBuffering/updated"
            | "account/login/completed"
            | "hook/started"
            | "hook/completed"
            | "mcpServer/oauthLogin/completed"
            | "windows/worldWritableWarning"
            | "windowsSandbox/setupCompleted"
            | "item/autoApprovalReview/started"
            | "item/autoApprovalReview/completed"
            | "thread/realtime/started"
            | "thread/realtime/closed"
            | "thread/realtime/error"
            | "thread/realtime/transcript"
            | "thread/realtime/sdp"
            | "item/tool/requestUserInput"
            | "mcpServer/elicitation/request"
            | "applyPatchApproval"
            | "execCommandApproval"
            | "account/chatgptAuthTokens/refresh"
            | "attestation/generate"
    )
}
pub(super) fn diagnostic(classification: &str) -> Vec<PublicWireFact> {
    vec![PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        },
    )]
}
pub(super) fn string<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field)?.as_str()
}
