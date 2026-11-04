use gent_testkit::host_absolute_path;

use crate::{
    claurst_local_readiness::{
        ClaurstLocalReadiness, ClaurstLocalReadinessError, ClaurstLocalReadinessService,
    },
    claurst_local_runtime::ClaurstLocalRuntimeRequest,
    local_model_catalog::LocalModelCatalog,
    local_model_provisioning::{
        LocalModelProvisioner, LocalModelProvisioningError, gguf::tests::chat_model,
    },
};

fn request(effort: gent_types::AgentChatEffort) -> ClaurstLocalRuntimeRequest {
    ClaurstLocalRuntimeRequest {
        claurst_executable: host_absolute_path("/opt/gent/bin/claurst"),
        llama_server_executable: host_absolute_path("/opt/gent/bin/llama-server"),
        model_path: host_absolute_path("/unresolved/model.gguf"),
        claurst_home: host_absolute_path("/opt/gent/claurst"),
        effort,
        mode: gent_types::AgentChatMode::Agent,
        permission_mode: gent_types::PermissionMode::AskEveryTime,
        mcp_servers: Vec::new(),
    }
}

#[test]
fn a_user_model_launches_its_file_in_place_with_only_derived_arguments() {
    let data = tempfile::tempdir().unwrap();
    let picked = tempfile::tempdir().unwrap();
    let path = picked.path().join("qwen3-custom.gguf");
    std::fs::write(&path, chat_model("Qwen3 Custom", 16_384).bytes()).unwrap();
    let provisioner =
        LocalModelProvisioner::new(data.path(), LocalModelCatalog::shipped().unwrap());
    let id = provisioner.add_user_model(path.to_str().unwrap()).unwrap();
    let readiness = ClaurstLocalReadinessService::new(provisioner.clone());
    for effort in [
        gent_types::AgentChatEffort::Low,
        gent_types::AgentChatEffort::Ultra,
    ] {
        let ClaurstLocalReadiness::Ready(plan) =
            readiness.assess(&id, request(effort), 18_080).unwrap()
        else {
            panic!("a present user model is ready");
        };
        let canonical = provisioner.model(&id).unwrap();
        let crate::local_model_catalog::LocalModelSource::UserFile { path: stored, .. } =
            canonical.source
        else {
            panic!("a user model record");
        };
        assert_eq!(plan.model_path, stored);
        let argv = &plan.llama_server.arguments;
        assert_eq!(argv[..2], ["-m".to_owned(), stored.display().to_string()]);
        let context = argv.iter().position(|arg| arg == "--ctx-size").unwrap();
        assert_eq!(argv[context + 1], "16384");
        assert!(argv.iter().any(|arg| arg == "--jinja"));
        for absent in [
            "--chat-template-file",
            "--chat-template-kwargs",
            "--reasoning",
            "--reasoning-budget",
        ] {
            assert!(!argv.iter().any(|arg| arg == absent), "{absent}: {argv:?}");
        }
        assert_eq!(plan.chat_template_path, None);
        assert!(plan.llama_server.environment.is_empty());
        assert_eq!(plan.model_configuration["modelId"], id.as_str());
        assert_eq!(plan.model_configuration["contextTokens"], 16_384);
    }
    assert_eq!(std::fs::read_dir(picked.path()).unwrap().count(), 1);
    assert!(!data.path().join("models").exists());
}

#[test]
fn launch_revalidates_a_moved_or_deleted_file_with_a_typed_error() {
    let data = tempfile::tempdir().unwrap();
    let path = data.path().join("model.gguf");
    std::fs::write(&path, chat_model("Movable", 8_192).bytes()).unwrap();
    let provisioner =
        LocalModelProvisioner::new(data.path(), LocalModelCatalog::shipped().unwrap());
    let id = provisioner.add_user_model(path.to_str().unwrap()).unwrap();
    let readiness = ClaurstLocalReadinessService::new(provisioner.clone());
    std::fs::rename(&path, data.path().join("moved.gguf")).unwrap();
    for _ in 0..2 {
        let error = readiness
            .assess(&id, request(gent_types::AgentChatEffort::Medium), 18_080)
            .unwrap_err();
        assert!(matches!(
            error,
            ClaurstLocalReadinessError::Provisioning(LocalModelProvisioningError::ModelFileMissing(
                ref missing
            )) if *missing == id
        ));
    }
    assert!(provisioner.model(&id).is_ok(), "the entry is never deleted");
    std::fs::rename(data.path().join("moved.gguf"), &path).unwrap();
    assert!(matches!(
        readiness.assess(&id, request(gent_types::AgentChatEffort::Medium), 18_080),
        Ok(ClaurstLocalReadiness::Ready(_))
    ));
}
