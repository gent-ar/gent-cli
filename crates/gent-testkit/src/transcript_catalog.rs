//! The required public-provider evidence dimensions.

/// Public providers whose recordings are required before authority transfer.
pub const PUBLIC_PROVIDERS: [&str; 2] = ["claude", "codex"];

/// Scenarios that prove the public-driver baseline instead of a single happy path.
pub const REQUIRED_SCENARIOS: [&str; 15] = [
    "full_turn",
    "tool_use",
    "tool_error",
    "thinking",
    "permission_prompt",
    "permission_persistent",
    "plan_mode",
    "subagent",
    "compaction",
    "mcp_tool",
    "resume",
    "interrupt",
    "steer",
    "usage_cost",
    "malformed_tolerance",
];
