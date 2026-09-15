use std::path::{Component, Path, PathBuf};

use gent_types::{
    AgentChatMode, PermissionCategory, PermissionDenialReason, PermissionMode, PermissionRequest,
    PolicyRecord,
};
use serde_json::Value;

use crate::permission_preflight::PermissionPreflight;

const PATH_KEYS: [&str; 3] = ["file_path", "path", "notebook_path"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClaurstPermissionDecision {
    Allow,
    Ask,
    Deny(Option<PermissionDenialReason>),
}

pub(crate) fn decide(
    mode: AgentChatMode,
    policy: &PolicyRecord,
    request: &PermissionRequest,
    workspace: &Path,
) -> ClaurstPermissionDecision {
    let changes = matches!(
        request.category,
        PermissionCategory::Edit | PermissionCategory::Command
    );
    if mode != AgentChatMode::Agent && changes {
        return ClaurstPermissionDecision::Deny(Some(PermissionDenialReason::ReadOnlyMode));
    }
    if request.category == PermissionCategory::Command {
        return match crate::permission_preflight::evaluate(policy, request) {
            PermissionPreflight::Allow => ClaurstPermissionDecision::Allow,
            PermissionPreflight::Deny => ClaurstPermissionDecision::Deny(None),
            PermissionPreflight::Ask => ClaurstPermissionDecision::Ask,
        };
    }
    if policy.allowed_tools.contains(&request.tool_name)
        || policy.allowed_categories.contains(&request.category)
    {
        return ClaurstPermissionDecision::Allow;
    }
    let input = request.input.as_ref();
    let bypass = policy.mode == PermissionMode::Bypass;
    let unattended = match (policy.mode, request.category) {
        (_, PermissionCategory::Read) => bypass || reads_stay_in(workspace, input),
        (PermissionMode::AskEveryTime, _) => false,
        (_, PermissionCategory::Edit) => edits_stay_in(workspace, input),
        (PermissionMode::Bypass, PermissionCategory::Network | PermissionCategory::Provider) => {
            true
        }
        _ => false,
    };
    if unattended {
        ClaurstPermissionDecision::Allow
    } else {
        ClaurstPermissionDecision::Ask
    }
}

fn reads_stay_in(workspace: &Path, input: Option<&Value>) -> bool {
    let pattern = input
        .and_then(|input| input.get("pattern"))
        .and_then(Value::as_str)
        .filter(|pattern| pattern.starts_with('/') || pattern.starts_with('~'));
    paths(input)
        .chain(pattern)
        .all(|path| inside(workspace, path))
}

fn edits_stay_in(workspace: &Path, input: Option<&Value>) -> bool {
    let mut paths = paths(input).peekable();
    paths.peek().is_some() && paths.all(|path| inside(workspace, path))
}

fn paths(input: Option<&Value>) -> impl Iterator<Item = &str> {
    PATH_KEYS.iter().filter_map(move |key| {
        input
            .and_then(|input| input.get(*key))
            .and_then(Value::as_str)
    })
}

fn inside(workspace: &Path, raw: &str) -> bool {
    if raw.starts_with('~') {
        return false;
    }
    let root = resolve(workspace);
    let path = Path::new(raw);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };
    resolve(&joined).starts_with(root)
}

fn resolve(path: &Path) -> PathBuf {
    let mut lexical = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                lexical.pop();
            }
            Component::CurDir => {}
            other => lexical.push(other),
        }
    }
    let mut existing = lexical.as_path();
    let mut missing = Vec::new();
    loop {
        if let Ok(canonical) = existing.canonicalize() {
            return missing
                .iter()
                .rev()
                .fold(canonical, |path, name| path.join(name));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_owned());
                existing = parent;
            }
            _ => return lexical,
        }
    }
}

#[cfg(test)]
#[path = "claurst_permission_policy_tests.rs"]
mod tests;
