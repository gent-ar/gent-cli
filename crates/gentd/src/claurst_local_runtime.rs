use std::{collections::BTreeMap, path::PathBuf};

use gent_types::{AgentChatEffort, AgentChatMode, PermissionMode};
use serde_json::json;

use crate::local_model_catalog::LocalModelRecord;

const LLAMA_CPP_PROVIDER: &str = "llama-cpp";
const DEFAULT_CONTEXT_SIZE: u32 = 8_192;

fn chat_template_contents(file: &str) -> Option<&'static str> {
    match file {
        "qwen2.5-tool-use.jinja" => Some(include_str!("../templates/qwen2.5-tool-use.jinja")),
        "hermes-3-tool-use.jinja" => Some(include_str!("../templates/hermes-3-tool-use.jinja")),
        _ => None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaurstLocalRuntimeRequest {
    pub(crate) claurst_executable: PathBuf,
    pub(crate) llama_server_executable: PathBuf,
    pub(crate) model_path: PathBuf,
    pub(crate) claurst_home: PathBuf,
    pub(crate) effort: AgentChatEffort,
    pub(crate) mode: AgentChatMode,
    pub(crate) permission_mode: PermissionMode,
    pub(crate) mcp_servers: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalProcessLaunch {
    pub(crate) executable: PathBuf,
    pub(crate) arguments: Vec<String>,
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaurstLocalRuntimePlan {
    pub(crate) model_path: PathBuf,
    pub(crate) server_url: String,
    pub(crate) settings_path: PathBuf,
    pub(crate) settings_json: String,
    pub(crate) chat_template_path: Option<PathBuf>,
    pub(crate) chat_template_contents: Option<String>,
    pub(crate) llama_server: LocalProcessLaunch,
    pub(crate) claurst_acp: LocalProcessLaunch,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub(crate) enum ClaurstLocalRuntimePlanError {
    #[error("the local Claurst runtime requires absolute {0}")]
    RelativePath(&'static str),
    #[error("the local Claurst runtime requires a non-zero port")]
    ZeroPort,
}

impl ClaurstLocalRuntimePlan {
    pub(crate) fn build(
        request: ClaurstLocalRuntimeRequest,
        model: &LocalModelRecord,
        port: u16,
    ) -> Result<Self, ClaurstLocalRuntimePlanError> {
        for (path, label) in [
            (&request.claurst_executable, "Claurst executable"),
            (&request.llama_server_executable, "llama.cpp executable"),
            (&request.model_path, "model file"),
            (&request.claurst_home, "Claurst home"),
        ] {
            if !path.is_absolute() {
                return Err(ClaurstLocalRuntimePlanError::RelativePath(label));
            }
        }
        if port == 0 {
            return Err(ClaurstLocalRuntimePlanError::ZeroPort);
        }

        let model_path = request.model_path;
        let server_url = format!("http://127.0.0.1:{port}");
        let settings_path = request.claurst_home.join(".claurst/settings.json");
        let chat_template_path = model
            .chat_template_file
            .as_deref()
            .map(|file| request.claurst_home.join(".claurst/templates").join(file));
        let chat_template_contents = model
            .chat_template_file
            .as_deref()
            .and_then(chat_template_contents)
            .map(str::to_owned);
        let settings_json = serde_json::to_string_pretty(&local_settings(
            model,
            request.effort,
            request.mode,
            request.permission_mode,
            request.mcp_servers,
        ))
        .expect("a fixed JSON settings document serializes");

        let llama_server = LocalProcessLaunch {
            executable: request.llama_server_executable,
            arguments: [
                "-m".into(),
                model_path.display().to_string(),
                "--host".into(),
                "127.0.0.1".into(),
                "--port".into(),
                port.to_string(),
                "--jinja".into(),
                "--ctx-size".into(),
                DEFAULT_CONTEXT_SIZE.to_string(),
                "--parallel".into(),
                "1".into(),
            ]
            .into_iter()
            .chain(
                chat_template_path
                    .as_ref()
                    .map(|path| vec!["--chat-template-file".into(), path.display().to_string()])
                    .unwrap_or_default(),
            )
            .collect(),
            environment: BTreeMap::new(),
        };
        let claurst_acp = LocalProcessLaunch {
            executable: request.claurst_executable,
            arguments: vec!["acp".into()],
            environment: BTreeMap::from([
                ("HOME".into(), request.claurst_home.display().to_string()),
                (
                    "CLAURST_HOME".into(),
                    request.claurst_home.join(".claurst").display().to_string(),
                ),
                ("LLAMA_CPP_HOST".into(), server_url.clone()),
            ]),
        };

        Ok(Self {
            model_path,
            server_url,
            settings_path,
            settings_json,
            chat_template_path,
            chat_template_contents,
            llama_server,
            claurst_acp,
        })
    }
}

fn local_settings(
    model: &LocalModelRecord,
    effort: AgentChatEffort,
    mode: AgentChatMode,
    permission_mode: PermissionMode,
    mcp_servers: Vec<serde_json::Value>,
) -> serde_json::Value {
    let provider_model = format!("{LLAMA_CPP_PROVIDER}/{}", model.provider_model_id);
    json!({
        "version": 1,
        "provider": LLAMA_CPP_PROVIDER,
        "config": {
            "api_key": null,
            "model": provider_model,
            "max_tokens": local_max_tokens(effort),
            "permission_mode": claurst_permission_mode(permission_mode),
            "theme": "default",
            "output_style": null,
            "auto_compact": true,
            "compact_threshold": 0.0,
            "verbose": false,
            "output_format": "text",
            "mcp_servers": mcp_servers,
            "lsp_servers": [],
            "allowed_tools": [],
            "disallowed_tools": [],
            "env": {},
            "enable_all_mcp_servers": false,
            "custom_system_prompt": system_instruction(model, effort, mode),
            "append_system_prompt": null,
            "disable_claude_mds": false,
            "project_dir": null,
            "workspace_paths": [],
            "additional_dirs": [],
            "hooks": {},
            "fileAutocompleteLimit": 0,
            "fileAutocompleteShowHiddenFiles": false,
            "fileInjectionEnabled": false,
            "fileInjectionMaxSize": 0,
            "provider": LLAMA_CPP_PROVIDER,
            "provider_configs": {},
            "formatter": {},
            "commands": {},
            "agents": {},
            "skills": { "paths": [], "urls": [] },
            "managed_agents": null
        },
        "providers": { LLAMA_CPP_PROVIDER: {} },
        "commands": {},
        "formatter": {},
        "agents": {},
        "skills": { "paths": [], "urls": [] },
        "managed_agents": null,
        "autoCopyOnHighlight": false,
        "showCwd": false,
        "showGitBranch": false,
        "notifications": false,
        "showTurnDuration": false,
        "reduceMotion": false,
        "terminalProgressBar": false,
        "autoCompact": false,
        "fileAutocompleteLimit": 0,
        "fileAutocompleteShowHiddenFiles": false,
        "fileInjectionEnabled": false,
        "fileInjectionMaxSize": 0
    })
}

fn claurst_permission_mode(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::Plan => "plan",
        PermissionMode::AutoAcceptEdits | PermissionMode::Autonomous => "accept-edits",
        PermissionMode::Bypass => "bypass-permissions",
    }
}

fn system_instruction(
    model: &LocalModelRecord,
    effort: AgentChatEffort,
    mode: AgentChatMode,
) -> String {
    format!(
        "{}{}",
        mode_instruction(mode),
        qwen3_effort_instruction(model, effort)
    )
}

fn mode_instruction(mode: AgentChatMode) -> &'static str {
    match mode {
        AgentChatMode::Ask => "Answer and explain. Do not invoke tools or change files.",
        AgentChatMode::Plan => {
            "Inspect only as needed, then provide a concrete plan. Do not change files or invoke destructive tools."
        }
        AgentChatMode::Agent => {
            "You are Gent, a local coding agent. Answer directly. Use available tools when needed. Request permission before actions that require it. Respect the workspace and MCP tools supplied for this session."
        }
    }
}

fn qwen3_effort_instruction(model: &LocalModelRecord, effort: AgentChatEffort) -> &'static str {
    if !model.id.starts_with("qwen3-") {
        return "";
    }
    match effort {
        AgentChatEffort::Low | AgentChatEffort::Medium => {
            "\n\n/no_think\nUse the non-thinking posture for this turn."
        }
        AgentChatEffort::High
        | AgentChatEffort::XHigh
        | AgentChatEffort::Max
        | AgentChatEffort::Ultra => {
            "\n\n/think\nUse the thinking posture for this turn, then act or answer."
        }
    }
}

fn local_max_tokens(effort: AgentChatEffort) -> u32 {
    match effort {
        AgentChatEffort::Low => 2_048,
        AgentChatEffort::Medium => 4_096,
        AgentChatEffort::High => 8_192,
        AgentChatEffort::XHigh => 12_288,
        AgentChatEffort::Max => 16_384,
        AgentChatEffort::Ultra => 24_576,
    }
}

#[cfg(test)]
#[path = "claurst_local_runtime_tests.rs"]
mod tests;
