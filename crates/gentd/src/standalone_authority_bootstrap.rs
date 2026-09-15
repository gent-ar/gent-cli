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
    let authority_release =
        crate::standalone_authority_release::StandaloneAuthorityRelease::resolve(
            args.standalone_authority_release.clone(),
            &args.standalone_authority_keys,
            &data_dir,
        )?;
    if args.verify_standalone_authority_release {
        authority_release
            .as_ref()
            .ok_or("standalone authority verification requires a signed release")?
            .load(startup::unix_seconds())?;
        return Ok(());
    }
    #[cfg(unix)]
    crate::private_paths::prepare_data_dir(&data_dir)?;
    #[cfg(windows)]
    std::fs::create_dir_all(&data_dir)?;
    let _host_lock = host_lock::acquire(&data_dir)?;
    let _provider_groups = daemon_bootstrap::stop_provider_groups_left_behind(&data_dir)?;
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
    let verified_release = authority_release.as_ref().and_then(|release| {
        release
            .load(startup::unix_seconds())
            .inspect_err(|error| eprintln!("gentd cannot use its signed provider release: {error}"))
            .ok()
            .map(|verified| (release, verified))
    });
    let compatibility = verified_release.as_ref().map_or_else(
        || CompatibilityAssessment::load(None, &[], startup::unix_seconds()),
        |(_, verified)| verified.compatibility(),
    );
    let capability_profile = standalone_capability_profile(verified_release.is_some());
    let reopened = data_dir.join("gent.db").is_file();
    let state = DaemonCompositionState::open(&data_dir, &capability_profile, compatibility)?;
    if reopened {
        state.fence_unclean_predecessor()?;
    }
    let executables = crate::provider_executables::ProviderExecutables::standalone(
        args.standalone_claude_executable.clone(),
        args.standalone_codex_executable.clone(),
        &data_dir,
        state.ledger().clone(),
        authority_release.clone(),
    );
    let authority = compose_standalone_authority(
        &state,
        &StandaloneAuthorityConfig {
            data_dir: data_dir.clone(),
            executables: executables.clone(),
            mcp_config: mcp_config.clone(),
        },
    )?;
    let prompt_provider_provision = verified_release
        .as_ref()
        .map(|(release, verified)| {
            crate::standalone_provider_provision::compose(&state, release, verified)
        })
        .transpose()?
        .map(|provision| {
            std::sync::Arc::new(
                crate::ordinary_lifecycle_cadence::wake::ProvisionedPromptWake::new(
                    provision,
                    authority.prompt_ingress(),
                ),
            )
                as std::sync::Arc<
                    dyn crate::prompt_provider_provision_boundary::PromptProviderProvisionPort,
                >
        });
    let claurst_runtime = claurst_runtime_config(&args, &data_dir, mcp_config)?;
    let gent_runtime = claurst_runtime.is_some();
    let doctor_mcp_servers = mcp_server_names.clone();
    authority
        .attach_lazy_claurst_runtime(claurst_runtime)
        .await?;
    let side_question_runners =
        crate::agent_chat_side_question_runners::AgentChatSideQuestionRunnerSources {
            data_dir: data_dir.clone(),
            executables: executables.clone(),
            claurst_bridge: authority.claurst_side_question_bridge(),
        };
    let provider_auth: std::sync::Arc<dyn crate::provider_auth_api::ProviderAuthPort> =
        std::sync::Arc::new(crate::provider_auth_api::StandaloneProviderAuthPort::new(
            executables.clone(),
        ));
    let model_catalog = crate::runtime_facade::model_catalog::compose_standalone(
        &data_dir,
        crate::runtime_facade::model_catalog::StandaloneModelCatalogConfig {
            local_models: authority.claurst_models().clone(),
            auth: std::sync::Arc::clone(&provider_auth),
            executables: executables.clone(),
        },
    );
    let provisioned = state.ledger().clone();
    let runtime = RuntimeFacade::from_state_with_standalone_authority(
        state,
        crate::runtime_update_config::packaged::current_executable_update_checks(
            startup::unix_seconds(),
        ),
        authority.prompt_ingress(),
        authority.claurst_models().clone(),
        Some(authority.provider_readiness_port()),
        prompt_provider_provision,
        Some(provider_auth),
        mcp_server_count,
        mcp_server_names,
        Some(side_question_runners),
    )?
    .with_model_catalog(model_catalog)
    .with_standalone_doctor(crate::dependency_catalog::standalone::StandaloneDoctor {
        executables,
        node: crate::node_runtime_lock::standalone_node_binary(),
        mcp_servers: doctor_mcp_servers,
        gent_runtime,
        provider_release: authority_release.is_some(),
        provisioned: Some(std::sync::Arc::new(provisioned)),
    });
    let recovered = authority.clone();
    let mut cadence = tokio::spawn(async move { authority.run_cadence().await });
    tokio::select! {
        result = &mut cadence => return lifecycle_stopped(result),
        ready = recovered.wait_until_ready() => ready?,
        () = daemon_bootstrap::terminated() => {
            cadence.abort();
            return Ok(());
        }
    }
    let serve = daemon_bootstrap::serve_ordinary(runtime, &args, &data_dir);
    tokio::pin!(serve);
    tokio::select! {
        result = &mut cadence => lifecycle_stopped(result),
        result = &mut serve => {
            cadence.abort();
            result
        }
        () = daemon_bootstrap::terminated() => {
            cadence.abort();
            Ok(())
        }
    }
}

fn lifecycle_stopped(
    result: Result<Result<(), String>, tokio::task::JoinError>,
) -> Result<(), Box<dyn std::error::Error>> {
    result.map_err(|_| "standalone provider lifecycle task failed")??;
    Err("standalone provider lifecycle stopped unexpectedly".into())
}

fn standalone_capability_profile(provider_provision: bool) -> RuntimeCapabilityProfile {
    let mut features = vec![
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::AgentChatProjection,
        RuntimeCapabilityFeature::ConversationActivity,
        RuntimeCapabilityFeature::AgentChatPermissions,
        RuntimeCapabilityFeature::TurnFollow,
        RuntimeCapabilityFeature::ReviewedPlans,
        RuntimeCapabilityFeature::ProviderReadiness,
        RuntimeCapabilityFeature::ProviderAuth,
        RuntimeCapabilityFeature::LocalModels,
        RuntimeCapabilityFeature::PromptTemplates,
        RuntimeCapabilityFeature::WorkspaceDocuments,
        RuntimeCapabilityFeature::WorkspaceGit,
        RuntimeCapabilityFeature::RuntimeUpdateCheck,
    ];
    if provider_provision {
        features.push(RuntimeCapabilityFeature::PromptProviderProvision);
    }
    RuntimeCapabilityProfile::new(features)
}

fn validate(args: &Args) -> Result<(), String> {
    validate_build(args, cfg!(debug_assertions))
}

fn validate_build(args: &Args, development_build: bool) -> Result<(), String> {
    if !development_build
        && (args.standalone_claude_executable.is_some()
            || args.standalone_codex_executable.is_some())
    {
        return Err(
            "explicit Claude and Codex executables require a development build of gentd".into(),
        );
    }
    if args.agent_chat_authority {
        return Err("standalone authority cannot be combined with another daemon authority".into());
    }
    if args.standalone_authority_release.is_some() != !args.standalone_authority_keys.is_empty() {
        return Err("standalone authority release and root keys must be supplied together".into());
    }
    if args.verify_standalone_authority_release && args.standalone_authority_release.is_none() {
        return Err("standalone authority verification requires a signed release".into());
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
            permission_mode: gent_types::PermissionMode::AskEveryTime,
            mcp_servers: Vec::new(),
        },
        mcp_config,
    }))
}

#[cfg(test)]
#[path = "standalone_authority_bootstrap_tests.rs"]
mod tests;
