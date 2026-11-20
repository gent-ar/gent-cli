use gent_types::PermissionCategory;

pub(crate) fn requires_user_response(tool_name: &str) -> bool {
    matches!(tool_name, "AskUserQuestion" | "ask_user")
}

pub(crate) const SESSION_INTERNAL_TOOLS: [&str; 20] = [
    "Agent",
    "Task",
    "ScheduleWakeup",
    "CronCreate",
    "CronDelete",
    "CronList",
    "TodoWrite",
    "TaskCreate",
    "TaskUpdate",
    "TaskGet",
    "TaskList",
    "TaskOutput",
    "TaskStop",
    "EnterPlanMode",
    "ExitPlanMode",
    "Skill",
    "BashOutput",
    "KillShell",
    "KillBash",
    "SendMessage",
];

pub(crate) fn stays_in_session(tool_name: &str) -> bool {
    SESSION_INTERNAL_TOOLS.contains(&tool_name)
}

pub(crate) fn for_tool(tool_name: &str) -> PermissionCategory {
    match tool_name {
        "Read"
        | "read_file"
        | "show_file"
        | "ReadFile"
        | "ReadManyFiles"
        | "Grep"
        | "grep"
        | "grep_search"
        | "search_file_content"
        | "SearchText"
        | "semantic_search"
        | "Glob"
        | "file_search"
        | "FindFiles"
        | "glob"
        | "list_directory"
        | "ReadFolder"
        | "ToolSearch" => PermissionCategory::Read,
        "Edit" | "Write" | "edit_file" | "write_file" | "WriteFile" | "apply_patch" | "replace"
        | "Replace" | "NotebookEdit" => PermissionCategory::Edit,
        "Bash" | "bash" | "PowerShell" | "powershell" | "pwsh" | "shell" | "run_in_terminal"
        | "run_shell_command" | "Shell" | "session.shell.exec" | "read_bash" | "write_bash"
        | "stop_bash" | "session.shell.kill" | "Command" | "Permission" => {
            PermissionCategory::Command
        }
        "WebSearch" | "web_search" | "google_web_search" | "GoogleSearch" | "WebFetch"
        | "web_fetch" => PermissionCategory::Network,
        _ => PermissionCategory::Provider,
    }
}

#[cfg(test)]
mod tests {
    use gent_types::PermissionCategory;

    use super::{SESSION_INTERNAL_TOOLS, for_tool, requires_user_response, stays_in_session};

    #[test]
    fn session_internal_tools_are_provider_controls_that_never_collect_an_answer() {
        for tool in SESSION_INTERNAL_TOOLS {
            assert!(stays_in_session(tool), "{tool}");
            assert_eq!(for_tool(tool), PermissionCategory::Provider, "{tool}");
            assert!(!requires_user_response(tool), "{tool}");
        }
        for tool in [
            "AskUserQuestion",
            "ask_user",
            "mcp__server__Agent",
            "mcp__gent-chat__gent_chat_create",
            "agent",
            "Bash",
            "read_bash",
            "stop_bash",
            "monitor",
            "Unknown",
        ] {
            assert!(!stays_in_session(tool), "{tool}");
        }
    }

    #[test]
    fn classifies_the_shared_provider_tool_vocabulary() {
        assert_eq!(for_tool("Read"), PermissionCategory::Read);
        assert_eq!(for_tool("Write"), PermissionCategory::Edit);
        assert_eq!(for_tool("Bash"), PermissionCategory::Command);
        assert_eq!(for_tool("PowerShell"), PermissionCategory::Command);
        assert_eq!(for_tool("WebFetch"), PermissionCategory::Network);
        assert_eq!(for_tool("Permission"), PermissionCategory::Command);
        assert_eq!(for_tool("mcp__server__tool"), PermissionCategory::Provider);
    }
}
