use super::{BTreeMap, Path, json};
pub(super) fn isolate_environment(environment: &mut BTreeMap<String, String>, root: &Path) {
    for name in ["TEMP", "TMP"] {
        environment.insert(name.into(), root.to_string_lossy().into_owned());
    }
    for name in [
        "DISABLE_AUTOUPDATER",
        "DISABLE_TELEMETRY",
        "DISABLE_ERROR_REPORTING",
        "CLAUDE_CODE_SKIP_PROMPT_HISTORY",
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
        "CLAUDE_CODE_DISABLE_OFFICIAL_MARKETPLACE_AUTOINSTALL",
        "CLAUDE_CODE_DISABLE_POLICY_SKILLS",
    ] {
        environment.insert(name.into(), "1".into());
    }
    environment.insert("ENABLE_CLAUDEAI_MCP_SERVERS".into(), "false".into());
    environment.insert(
        "CLAUDE_CODE_PLUGIN_CACHE_DIR".into(),
        root.join("plugins").to_string_lossy().into_owned(),
    );
    environment.insert(
        "CLAUDE_CODE_DEBUG_LOGS_DIR".into(),
        root.join("debug").to_string_lossy().into_owned(),
    );
}

pub(super) fn arguments() -> Vec<String> {
    let settings = json!({"disableAllHooks":true,"enabledPlugins":{
        "cc-plugin-agents-md@builtin":false,"cc-plugin-plugin-authoring@builtin":false,
        "cc-plugin-telemetry@builtin":false,"cc-plugin-you-should-know@builtin":false}})
    .to_string();
    [
        "--safe-mode",
        "--restricted",
        "--disable-slash-commands",
        "--setting-sources",
        "",
        "--settings",
        &settings,
        "--tools",
        "",
        "--disallowedTools",
        "mcp__*",
        "--strict-mcp-config",
        "--mcp-config",
        "{\"mcpServers\":{}}",
        "--max-turns",
        "1",
        "--no-session-persistence",
        "--permission-mode",
        "dontAsk",
        "--no-chrome",
        "--system-prompt",
        "Credential transport validation. Return OK.",
        "--model",
        "haiku",
        "--print",
        "--output-format",
        "stream-json",
        "--verbose",
        "Gent credential probe. Reply OK.",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}
