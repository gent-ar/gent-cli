use gent_runtime::catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile};

use crate::{
    compatibility_assessment::CompatibilityAssessment,
    daemon_bootstrap::{self, Args},
    host_lock,
    runtime_facade::{DaemonCompositionState, RuntimeFacade},
    standalone_authority_composition::{StandaloneAuthorityConfig, compose_standalone_authority},
    standalone_claurst_runtime_factory::StandaloneClaurstRuntimeConfig,
    startup,
};

pub(crate) async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    validate(&args)?;
    let data_dir = args
        .data_dir
        .clone()
        .unwrap_or_else(startup::default_data_dir);
    #[cfg(unix)]
    crate::private_paths::prepare_data_dir(&data_dir)?;
    #[cfg(windows)]
    std::fs::create_dir_all(&data_dir)?;
    let _host_lock = host_lock::acquire(&data_dir)?;
    let mcp_config = args
        .mcp_config
        .as_deref()
        .map(crate::standalone_mcp_config::StandaloneMcpConfig::load)
        .transpose()?;
    let mcp_config = match mcp_config {
        Some(config) => Some(config.with_internal_servers(&data_dir)?),
        None => Some(crate::standalone_mcp_config::StandaloneMcpConfig::internal_only(&data_dir)?),
    };
    let mcp_server_count = mcp_config
        .as_ref()
        .map(crate::standalone_mcp_config::StandaloneMcpConfig::server_count)
        .transpose()?
        .unwrap_or_default();
    let mcp_server_names = mcp_config
        .as_ref()
        .map(crate::standalone_mcp_config::StandaloneMcpConfig::server_names)
        .transpose()?
        .unwrap_or_default();

    let capability_profile = standalone_capability_profile();
    let reopened = data_dir.join("gent.db").is_file();
    let state = DaemonCompositionState::open(
        &data_dir,
        &capability_profile,
        CompatibilityAssessment::load(None, &[], startup::unix_seconds()),
    )?;
    if reopened {
        state.fence_unclean_predecessor()?;
    }
    let authority = compose_standalone_authority(
        &state,
        &StandaloneAuthorityConfig {
            data_dir: data_dir.clone(),
            claude_executable: args.standalone_claude_executable.clone(),
            codex_executable: args.standalone_codex_executable.clone(),
            mcp_config: mcp_config.clone(),
        },
    )?;
    authority
        .attach_lazy_claurst_runtime(claurst_runtime_config(&args, &data_dir, mcp_config)?)
        .await?;
    let side_question_runners =
        crate::agent_chat_side_question_runners::AgentChatSideQuestionRunnerSources {
            data_dir: data_dir.clone(),
            claude_executable: args.standalone_claude_executable.clone(),
            codex_executable: args.standalone_codex_executable.clone(),
            claurst_bridge: authority.claurst_side_question_bridge(),
        };
    let runtime = RuntimeFacade::from_state_with_standalone_authority(
        state,
        None,
        authority.prompt_ingress(),
        authority.claurst_models().clone(),
        mcp_server_count,
        mcp_server_names,
        Some(side_question_runners),
    )?;
    let readiness_authority = authority.clone();
    let mut cadence = tokio::spawn(async move { authority.run_cadence().await });
    tokio::select! {
        result = &mut cadence => {
            let result = result.map_err(|_| "standalone provider lifecycle task failed")?;
            result?;
            return Err("standalone provider lifecycle stopped before it became ready".into());
        }
        result = readiness_authority.wait_until_ready() => result?,
    }
    daemon_bootstrap::serve_ordinary(runtime, &args, &data_dir).await
}

fn standalone_capability_profile() -> RuntimeCapabilityProfile {
    RuntimeCapabilityProfile::new([
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::ConversationActivity,
        RuntimeCapabilityFeature::AgentChatPermissions,
        RuntimeCapabilityFeature::TurnFollow,
        RuntimeCapabilityFeature::ReviewedPlans,
        RuntimeCapabilityFeature::LocalModels,
        RuntimeCapabilityFeature::PromptTemplates,
        RuntimeCapabilityFeature::WorkspaceDocuments,
        RuntimeCapabilityFeature::WorkspaceGit,
    ])
}

fn validate(args: &Args) -> Result<(), String> {
    if args.agent_chat_authority {
        return Err("standalone authority cannot be combined with another daemon authority".into());
    }
    if args.standalone_claude_executable.is_some() != args.standalone_codex_executable.is_some() {
        return Err(
            "standalone Claude and Codex executable paths must be supplied together".into(),
        );
    }
    for (label, path) in [
        ("Claurst", args.standalone_claurst_executable.as_ref()),
        (
            "llama.cpp llama-server",
            args.standalone_llama_server_executable.as_ref(),
        ),
    ] {
        if let Some(path) = path
            && !std::fs::metadata(path)
                .map(|metadata| metadata.is_file())
                .unwrap_or(false)
        {
            return Err(format!("standalone {label} executable is not a file"));
        }
    }
    if args.standalone_claurst_executable.is_some()
        != args.standalone_llama_server_executable.is_some()
    {
        return Err(
            "standalone Claurst and llama.cpp executable paths must be supplied together".into(),
        );
    }
    Ok(())
}

fn claurst_runtime_config(
    args: &Args,
    data_dir: &std::path::Path,
    mcp_config: Option<crate::standalone_mcp_config::StandaloneMcpConfig>,
) -> Result<Option<StandaloneClaurstRuntimeConfig>, String> {
    let runtime = match (
        args.standalone_claurst_executable.clone(),
        args.standalone_llama_server_executable.clone(),
    ) {
        (Some(claurst_executable), Some(llama_server_executable)) => {
            Some(crate::packaged_claurst_runtime::PackagedClaurstRuntime {
                claurst_executable,
                llama_server_executable,
            })
        }
        (None, None) => {
            crate::packaged_claurst_runtime::PackagedClaurstRuntime::from_current_executable()?
        }
        _ => {
            return Err(
                "standalone Claurst and llama.cpp executable paths must be supplied together"
                    .into(),
            );
        }
    };
    let Some(runtime) = runtime else {
        return Ok(None);
    };
    Ok(Some(StandaloneClaurstRuntimeConfig {
        request: crate::claurst_local_runtime::ClaurstLocalRuntimeRequest {
            claurst_executable: runtime.claurst_executable,
            llama_server_executable: runtime.llama_server_executable,
            model_path: data_dir.join("models").join("unresolved.gguf"),
            claurst_home: data_dir.join("claurst"),
            effort: gent_types::AgentChatEffort::Medium,
            mode: gent_types::AgentChatMode::Agent,
            permission_mode: gent_types::PermissionMode::Default,
            mcp_servers: Vec::new(),
        },
        mcp_config,
    }))
}

#[cfg(test)]
#[path = "standalone_authority_bootstrap_tests.rs"]
mod tests;
