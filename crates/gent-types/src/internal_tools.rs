//! Single catalog for daemon-injected Gent MCP servers and their exact tools.
use serde_json::{Value, json};

pub const INTERNAL_SERVER_NAMES: [&str; 4] =
    ["gent-automations", "gent-chat", "gent-forge", "gent-goal"];

const CREATE_DESCRIPTION: &str = "Start a new Gent chat that works alongside this one. \
The new chat is a real conversation the user can open from the sidebar, with its own transcript \
and its own agent; it is not a background job and not a sub-agent. Give it a short label the user \
will recognize, and a prompt describing the work it should begin immediately. Returns its \
conversationId: keep it, because gent_chat_send and gent_chat_wait take it.";

const SEND_DESCRIPTION: &str = "Say something to another Gent chat you created or were created by. \
This is the only way to talk to it. If that chat is idle the message starts its next turn; if it \
is already working the message is steered into the turn in flight. Returns how it was delivered.";

const WAIT_DESCRIPTION: &str = "Block until the named Gent chats finish what they are doing, or \
until timeoutSeconds elapses. This call does not return while they are still working, so do not \
call it before you have given them something to do. On timeout it returns honestly, reporting \
each chat's phase and timedOut true, rather than failing. Returns each chat's final phase and the \
last thing it said.";

const LIST_DESCRIPTION: &str = "List the Gent chats in this conversation's thread: the chat that \
created this one, if any, and every chat this one created, with each one's label, current phase \
and last activity time. Use it to recover conversationIds after a restart.";

fn chat_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "gent_chat_create",
            "description": CREATE_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "required": ["label"],
                "properties": {
                    "label": {"type": "string", "description": "Short name the user sees for the new chat"},
                    "prompt": {"type": "string", "description": "What the new chat should start working on"},
                    "workspacePath": {"type": "string", "description": "Workspace for the new chat [default: this conversation's workspace]"},
                    "conversationId": {"type": "string", "description": "Only needed when gentd cannot resolve the calling conversation"}
                }
            }
        }),
        json!({
            "name": "gent_chat_send",
            "description": SEND_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "required": ["targetConversationId", "message"],
                "properties": {
                    "targetConversationId": {"type": "string"},
                    "message": {"type": "string"},
                    "conversationId": {"type": "string", "description": "Only needed when gentd cannot resolve the calling conversation"}
                }
            }
        }),
        json!({
            "name": "gent_chat_wait",
            "description": WAIT_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "required": ["conversationIds"],
                "properties": {
                    "conversationIds": {"type": "array", "items": {"type": "string"}, "maxItems": 16},
                    "timeoutSeconds": {"type": "integer", "minimum": 1, "maximum": 900},
                    "conversationId": {"type": "string", "description": "Only needed when gentd cannot resolve the calling conversation"}
                }
            }
        }),
        json!({
            "name": "gent_chat_list",
            "description": LIST_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "conversationId": {"type": "string", "description": "Only needed when gentd cannot resolve the calling conversation"}
                }
            }
        }),
    ]
}

pub fn tools(domain: Option<&str>) -> Vec<Value> {
    let mut all = vec![
        json!({"name":"gent_automations_list","description":"List Gent automations for a workspace","inputSchema":{"type":"object","required":["workspaceId"],"properties":{"workspaceId":{"type":"string"}}}}),
        json!({"name":"gent_automation_run","description":"Run one manual Gent automation","inputSchema":{"type":"object","required":["automationId"],"properties":{"automationId":{"type":"string"}}}}),
        json!({"name":"gent_forge_list","description":"List Gent Forge connectors for a workspace","inputSchema":{"type":"object","required":["workspaceId"],"properties":{"workspaceId":{"type":"string"}}}}),
        json!({"name":"gent_goal_update","description":"Report that the active Gent goal is complete, or blocked until the user helps","inputSchema":{"type":"object","required":["goalId","status"],"properties":{"goalId":{"type":"string"},"status":{"type":"string","enum":["complete","blocked"]},"note":{"type":"string"}}}}),
    ];
    all.extend(chat_tools());
    all.into_iter()
        .filter(|tool| {
            domain.is_none()
                || domain == Some("automations")
                    && tool["name"]
                        .as_str()
                        .is_some_and(|name| name.starts_with("gent_automation"))
                || domain == Some("forge") && tool["name"] == "gent_forge_list"
                || domain == Some("goal") && tool["name"] == "gent_goal_update"
                || domain == Some("chat")
                    && tool["name"]
                        .as_str()
                        .is_some_and(|name| name.starts_with("gent_chat_"))
        })
        .collect()
}

/// SDK callback aliases are reserved; they are never separate external sources.
pub fn reserved_server(name: &str) -> bool {
    INTERNAL_SERVER_NAMES
        .iter()
        .any(|server| name == *server || name == server.replace('-', "_"))
}

pub fn sdk_alias(name: &str) -> bool {
    reserved_server(name) && !INTERNAL_SERVER_NAMES.contains(&name)
}

/// Parse only an exact known tool belonging to an exact internal server.
pub fn callback_tool(name: &str) -> Option<(&'static str, String)> {
    let (server, tool) = name.strip_prefix("mcp__")?.split_once("__")?;
    let canonical = INTERNAL_SERVER_NAMES
        .into_iter()
        .find(|candidate| server == *candidate || server == candidate.replace('-', "_"))?;
    let domain = canonical.strip_prefix("gent-")?;
    tools(Some(domain))
        .iter()
        .any(|entry| entry["name"].as_str() == Some(tool))
        .then(|| (canonical, tool.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callbacks_require_exact_catalog_membership_and_aliases_are_reserved() {
        for server in INTERNAL_SERVER_NAMES {
            assert!(reserved_server(server));
            assert!(sdk_alias(&server.replace('-', "_")));
            for tool in tools(server.strip_prefix("gent-")) {
                let tool = tool["name"].as_str().unwrap();
                for spelling in [server.to_owned(), server.replace('-', "_")] {
                    assert!(callback_tool(&format!("mcp__{spelling}__{tool}")).is_some());
                }
            }
        }
        for forged in [
            "mcp__gent_goal__anything",
            "mcp__gent-goal__gent_chat_list",
            "mcp__other__gent_goal_update",
            "mcp__gent-goal__gent_goal_update__extra",
        ] {
            assert!(callback_tool(forged).is_none());
        }
    }
}
