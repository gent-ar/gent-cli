use gent_types::PermissionCategory;

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
        "Bash" | "bash" | "shell" | "run_in_terminal" | "run_shell_command" | "Shell"
        | "session.shell.exec" | "read_bash" | "write_bash" | "stop_bash"
        | "session.shell.kill" | "Command" => PermissionCategory::Command,
        "WebSearch" | "web_search" | "google_web_search" | "GoogleSearch" | "WebFetch"
        | "web_fetch" => PermissionCategory::Network,
        _ => PermissionCategory::Provider,
    }
}

#[cfg(test)]
mod tests {
    use gent_types::PermissionCategory;

    use super::for_tool;

    #[test]
    fn classifies_the_shared_provider_tool_vocabulary() {
        assert_eq!(for_tool("Read"), PermissionCategory::Read);
        assert_eq!(for_tool("Write"), PermissionCategory::Edit);
        assert_eq!(for_tool("Bash"), PermissionCategory::Command);
        assert_eq!(for_tool("WebFetch"), PermissionCategory::Network);
        assert_eq!(for_tool("mcp__server__tool"), PermissionCategory::Provider);
    }
}
