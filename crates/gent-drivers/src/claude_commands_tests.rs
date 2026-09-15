use gent_types::{AgentChatCommandDispatch, AgentChatCommandOrigin, AgentChatProvider};
use serde_json::json;

use super::{ClaudeBuiltin, claude_builtin, claude_command_descriptors};

#[test]
fn reported_commands_are_classified_as_builtins_skills_or_gent_reserved() {
    let initialize = json!({
        "models": [],
        "commands": [
            {"name": "context", "description": "Show context usage", "argumentHint": ""},
            {"name": "mcp", "description": "Manage MCP servers", "argumentHint": "[reconnect]"},
            {"name": "clear", "description": "Clear", "aliases": ["new", "reset"]},
            {"name": "__remote-workflow", "description": "hidden"},
            {"name": "my-skill", "description": "Project skill\nsecond line", "aliases": ["mine", "bad name"]},
            {"name": "bad name", "description": "invalid"}
        ]
    });
    let commands = claude_command_descriptors(&initialize).unwrap();
    let names: Vec<&str> = commands
        .iter()
        .map(|command| command.name.as_str())
        .collect();
    assert_eq!(names, ["context", "mcp", "my-skill"]);
    let provider = AgentChatProvider::Claude;
    assert_eq!(
        commands[0].origin,
        AgentChatCommandOrigin::ProviderBuiltin { provider }
    );
    assert_eq!(
        commands[0].dispatch,
        AgentChatCommandDispatch::ProviderNative
    );
    assert_eq!(commands[0].argument_hint, None);
    assert!(commands[0].availability.blocked_while_turn_active);
    assert_eq!(
        commands[1].dispatch,
        AgentChatCommandDispatch::Unsupported {
            reason: "MCP servers are configured through Gent.".into(),
            use_instead: Some("/tools".into()),
        }
    );
    assert!(!commands[1].availability.requires_conversation);
    assert_eq!(
        commands[2].origin,
        AgentChatCommandOrigin::ProviderSkill { provider }
    );
    assert_eq!(commands[2].description, "Project skill");
    assert_eq!(commands[2].aliases, ["mine"]);
    assert!(commands.iter().all(|command| command.is_valid()));
}

#[test]
fn a_response_without_commands_is_not_a_listing() {
    assert!(claude_command_descriptors(&json!({"models": []})).is_none());
    assert_eq!(claude_builtin("clear"), Some(ClaudeBuiltin::GentReserved));
    assert_eq!(claude_builtin("unlisted-skill"), None);
}
