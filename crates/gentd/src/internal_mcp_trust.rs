use serde_json::Value;

pub(crate) fn owned_chat_tools(
    settings: &[Value],
    injection: Option<&crate::standalone_mcp_config::InjectedInternalMcp>,
    conversation: &str,
    disallowed: &[String],
) -> Vec<String> {
    let Some(injection) = injection else {
        return Vec::new();
    };
    let actual = Value::Object(
        settings
            .iter()
            .cloned()
            .filter_map(|mut entry| {
                let name = entry.as_object_mut()?.remove("name")?.as_str()?.to_owned();
                Some((name, entry))
            })
            .collect(),
    );
    gent_types::internal_tools::tools(Some("chat"))
        .iter()
        .filter_map(|tool| {
            let tool = tool["name"].as_str()?;
            let name = format!(
                "mcp__{}__{tool}",
                crate::standalone_mcp_config::CHAT_SERVER_NAME
            );
            permits_expected(
                &actual,
                injection.servers(),
                conversation,
                &name,
                disallowed,
            )
            .then_some(name)
        })
        .collect()
}

pub(crate) fn permits(
    actual: Option<&Value>,
    injection: Option<&crate::standalone_mcp_config::InjectedInternalMcp>,
    conversation: &str,
    name: &str,
    disallowed: &[String],
) -> bool {
    let (Some(actual), Some(injection)) = (actual, injection) else {
        return false;
    };
    permits_expected(actual, injection.servers(), conversation, name, disallowed)
}

pub(super) fn permits_expected(
    actual: &Value,
    expected: &Value,
    conversation: &str,
    name: &str,
    disallowed: &[String],
) -> bool {
    if conversation.trim().is_empty() {
        return false;
    }
    let Some((server, tool)) = gent_types::internal_tools::callback_tool(name) else {
        return false;
    };
    let canonical = format!("mcp__{server}__{tool}");
    let alias = format!("mcp__{}__{tool}", server.replace('-', "_"));
    if disallowed
        .iter()
        .any(|rule| matches_rule(rule, &canonical) || matches_rule(rule, &alias))
    {
        return false;
    }
    if actual.get(server.replace('-', "_")).is_some() {
        return false;
    }
    let expected = crate::conversation_scoped_mcp::scope_server_map(expected, conversation);
    match (actual.get(server), expected.get(server)) {
        (Some(actual), Some(expected)) => actual == expected,
        _ => false,
    }
}

fn matches_rule(rule: &str, name: &str) -> bool {
    let mut rest = name;
    let mut parts = rule.split('*').peekable();
    let Some(first) = parts.next() else {
        return false;
    };
    let Some(next) = rest.strip_prefix(first) else {
        return false;
    };
    rest = next;
    if parts.peek().is_none() {
        return rest.is_empty();
    }
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return rest.ends_with(part);
        }
        let Some(index) = rest.find(part) else {
            return false;
        };
        rest = &rest[index + part.len()..];
    }
    false
}

#[cfg(test)]
#[path = "internal_mcp_trust_tests.rs"]
mod tests;
