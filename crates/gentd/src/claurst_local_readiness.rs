use crate::{
    claurst_local_runtime::{
        ClaurstLocalRuntimePlan, ClaurstLocalRuntimePlanError, ClaurstLocalRuntimeRequest,
    },
    local_model_provisioning::{
        LocalModelDownloadPlan, LocalModelProvisioner, LocalModelProvisioningError,
        ModelInstallState,
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClaurstLocalReadiness {
    DownloadRequired {
        plan: LocalModelDownloadPlan,
        downloaded_bytes: u64,
    },
    Ready(ClaurstLocalRuntimePlan),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ClaurstLocalReadinessError {
    #[error(transparent)]
    Provisioning(#[from] LocalModelProvisioningError),
    #[error(transparent)]
    RuntimePlan(#[from] ClaurstLocalRuntimePlanError),
}

#[derive(Clone, Debug)]
pub(crate) struct ClaurstLocalReadinessService {
    provisioner: LocalModelProvisioner,
}

impl ClaurstLocalReadinessService {
    #[must_use]
    pub(crate) const fn new(provisioner: LocalModelProvisioner) -> Self {
        Self { provisioner }
    }

    pub(crate) fn assess(
        &self,
        model_id: &str,
        request: ClaurstLocalRuntimeRequest,
        port: u16,
    ) -> Result<ClaurstLocalReadiness, ClaurstLocalReadinessError> {
        let plan = self.provisioner.plan(model_id)?;
        match self.provisioner.state(model_id)? {
            ModelInstallState::NotInstalled => Ok(ClaurstLocalReadiness::DownloadRequired {
                plan,
                downloaded_bytes: 0,
            }),
            ModelInstallState::Downloading { downloaded_bytes } => {
                Ok(ClaurstLocalReadiness::DownloadRequired {
                    plan,
                    downloaded_bytes,
                })
            }
            ModelInstallState::Ready { path } => {
                let mut request = request;
                request.model_path = path;
                let model = self
                    .provisioner
                    .model(model_id)
                    .expect("a successful curated download plan has its model record");
                Ok(ClaurstLocalReadiness::Ready(
                    ClaurstLocalRuntimePlan::build(request, model, port)?,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::{ClaurstLocalReadiness, ClaurstLocalReadinessService};
    use crate::{
        claurst_local_runtime::ClaurstLocalRuntimeRequest, local_model_catalog::LocalModelCatalog,
        local_model_provisioning::LocalModelProvisioner,
    };

    fn request() -> ClaurstLocalRuntimeRequest {
        ClaurstLocalRuntimeRequest {
            claurst_executable: PathBuf::from("/opt/gent/bin/claurst"),
            llama_server_executable: PathBuf::from("/opt/gent/bin/llama-server"),
            model_path: PathBuf::from("/untrusted/model.gguf"),
            claurst_home: PathBuf::from("/opt/gent/claurst"),
            effort: gent_types::AgentChatEffort::Medium,
            mode: gent_types::AgentChatMode::Agent,
            permission_mode: gent_types::PermissionMode::Default,
            mcp_servers: Vec::new(),
        }
    }

    fn service() -> (
        tempfile::TempDir,
        ClaurstLocalReadinessService,
        LocalModelProvisioner,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let catalog = LocalModelCatalog::from_json(
            r#"{"models":[{"id":"qwen2-5-coder-7b-instruct-q4-k-m","label":"Model","huggingface_url":"https://huggingface.co/gent/model/resolve/0123456789abcdef0123456789abcdef01234567/model.gguf","local_filename":"model.gguf","provider_model_id":"model","size_bytes":5,"sha256":"36bbe50ed96841d10443bcb670d6554f0a34b761be67ec9c4a8ad2c0c44ca42c"}]}"#,
        )
        .unwrap();
        let provisioner = LocalModelProvisioner::new(directory.path(), catalog);
        (
            directory,
            ClaurstLocalReadinessService::new(provisioner.clone()),
            provisioner,
        )
    }

    #[test]
    fn missing_or_partial_curated_model_returns_only_its_approved_download_plan() {
        let (_directory, readiness, provisioner) = service();
        let plan = provisioner
            .plan("qwen2-5-coder-7b-instruct-q4-k-m")
            .unwrap();
        assert!(matches!(
            readiness.assess("qwen2-5-coder-7b-instruct-q4-k-m", request(), 18_080).unwrap(),
            ClaurstLocalReadiness::DownloadRequired { downloaded_bytes: 0, plan: actual }
                if actual == plan
        ));
        provisioner.ensure_storage(&plan).unwrap();
        fs::write(&plan.partial_destination, [7_u8; 4]).unwrap();
        assert!(matches!(
            readiness
                .assess("qwen2-5-coder-7b-instruct-q4-k-m", request(), 18_080)
                .unwrap(),
            ClaurstLocalReadiness::DownloadRequired {
                downloaded_bytes: 4,
                ..
            }
        ));
    }

    #[test]
    fn exact_ready_model_produces_a_llama_cpp_plan_with_the_catalog_owned_path() {
        let (_directory, readiness, provisioner) = service();
        let plan = provisioner
            .plan("qwen2-5-coder-7b-instruct-q4-k-m")
            .unwrap();
        provisioner.ensure_storage(&plan).unwrap();
        fs::write(&plan.destination, b"abcde").unwrap();
        let ClaurstLocalReadiness::Ready(runtime) = readiness
            .assess("qwen2-5-coder-7b-instruct-q4-k-m", request(), 18_080)
            .unwrap()
        else {
            panic!("exact local GGUF must be ready");
        };
        assert_eq!(runtime.model_path, plan.destination);
        assert!(
            runtime
                .llama_server
                .arguments
                .contains(&runtime.model_path.display().to_string())
        );
        assert!(
            !runtime
                .llama_server
                .arguments
                .contains(&"/untrusted/model.gguf".into())
        );
    }
}
