use std::{collections::BTreeMap, path::PathBuf};

use gent_types::{AgentChatEffort, AgentChatMode, PermissionMode};
use serde_json::json;

use crate::{claurst_runtime_factory::LlamaSummaryEndpoint, local_model_catalog::LocalModelRecord};

const LLAMA_CPP_PROVIDER: &str = "llama-cpp";
const KV_CACHE_TYPE: &str = "q8_0";

fn chat_template_contents(file: &str) -> Option<&'static str> {
    match file {
        "qwen2.5-tool-use.jinja" => Some(include_str!("../templates/qwen2.5-tool-use.jinja")),
        "hermes-3-tool-use.jinja" => Some(include_str!("../templates/hermes-3-tool-use.jinja")),
        "qwen3-tool-use.jinja" => Some(include_str!("../templates/qwen3-tool-use.jinja")),
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
    pub(crate) working_directory: Option<PathBuf>,
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
    pub(crate) history_input_bytes: usize,
    pub(crate) summary_endpoint: LlamaSummaryEndpoint,
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
        let effort = request.effort;
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
            effort,
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
                model.context_tokens.to_string(),
                "--cache-type-k".into(),
                KV_CACHE_TYPE.into(),
                "--cache-type-v".into(),
                KV_CACHE_TYPE.into(),
                "--parallel".into(),
                "1".into(),
            ]
            .into_iter()
            .chain(profile::qwen3_reasoning_arguments(model, effort))
            .chain(
                chat_template_path
                    .as_ref()
                    .map(|path| {
                        vec![
                            "--chat-template-file".into(),
                            path.display().to_string(),
                            "--chat-template-kwargs".into(),
                            profile::template_kwargs(model, effort, request.mode),
                        ]
                    })
                    .unwrap_or_default(),
            )
            .collect(),
            environment: BTreeMap::new(),
            working_directory: None,
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
            working_directory: None,
        };

        let history_input_bytes = profile::history_input_bytes(model, effort);
        Ok(Self {
            summary_endpoint: summary_endpoint(model, &server_url, history_input_bytes),
            model_path,
            server_url,
            settings_path,
            settings_json,
            chat_template_path,
            chat_template_contents,
            llama_server,
            claurst_acp,
            history_input_bytes,
        })
    }
}

fn summary_endpoint(
    model: &LocalModelRecord,
    server_url: &str,
    history_input_bytes: usize,
) -> LlamaSummaryEndpoint {
    LlamaSummaryEndpoint {
        server_url: server_url.to_owned(),
        model: model.provider_model_id.clone(),
        context_tokens: model.context_tokens,
        history_input_bytes,
    }
}

fn local_settings(
    model: &LocalModelRecord,
    effort: AgentChatEffort,
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
            "max_tokens": profile::local_max_tokens(effort),
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
            "custom_system_prompt": null,
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
        PermissionMode::AskEveryTime => "default",
        PermissionMode::AutoAcceptEdits | PermissionMode::Autonomous => "acceptEdits",
        PermissionMode::Bypass => "bypassPermissions",
    }
}

#[path = "claurst_local_profile.rs"]
mod profile;

#[cfg(test)]
#[path = "claurst_local_runtime_tests.rs"]
mod tests;
