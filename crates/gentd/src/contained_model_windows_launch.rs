use super::super::{ModelEndpoint, ModelError, ModelResources, scope};
use super::{
    LaunchIntent, ModelRun, OpenOptions, Path, ProviderLaunch, SandboxLaunchProfile,
    SandboxResourceGrant, SandboxResourceLimits, SandboxWorkspaceAccess, SelectedModel, Sha256,
    WindowsModelFactory, no_reparse, pin,
};
use gent_drivers::SandboxedProviderLaunch;
use gent_types::{SandboxNetworkPolicy, SandboxedLaunchRequest};
use sha2::Digest;
use std::io::Write;
use std::os::windows::fs::OpenOptionsExt;
use std::sync::Arc;
impl WindowsModelFactory {
    pub(crate) fn launch(
        self,
        selected: SelectedModel,
        run: ModelRun,
        temporary_parent: &Path,
        mut config: serde_json::Value,
    ) -> Result<ModelEndpoint, ModelError> {
        if !admissible(&run, &selected, &config) {
            return Err(ModelError::Request);
        }
        config["modelPath"] = serde_json::json!(selected.path);
        let (_, sdk_pin) = pin(
            Path::new(&run.sdk_lock.canonical_path),
            &run.sdk_lock.digest_sha256,
        )?;
        let root = tempfile::Builder::new()
            .prefix("model-scope-")
            .tempdir_in(temporary_parent)
            .map_err(|_| ModelError::Request)?;
        let root_path = root
            .path()
            .canonicalize()
            .map_err(|_| ModelError::Request)?;
        for write in &run.writable_roots {
            let write = no_reparse(write)?;
            if root_path.starts_with(&write) || write.starts_with(&root_path) {
                return Err(ModelError::Request);
            }
        }
        let mut template_pin = None;
        if let Some(name) = &selected.record.chat_template_file {
            let contents = match name.as_str() {
                "qwen2.5-tool-use.jinja" => include_str!("../templates/qwen2.5-tool-use.jinja"),
                "hermes-3-tool-use.jinja" => include_str!("../templates/hermes-3-tool-use.jinja"),
                "qwen3-tool-use.jinja" => include_str!("../templates/qwen3-tool-use.jinja"),
                _ => return Err(ModelError::Request),
            };
            let path = root_path.join("chat-template.jinja");
            write_new(&path, contents.as_bytes())?;
            let hash = format!("{:x}", Sha256::digest(contents.as_bytes()));
            let (path, file) = pin(&path, &hash)?;
            config["templatePath"] = serde_json::json!(path);
            config["templateSha256"] = serde_json::json!(hash);
            template_pin = Some(file);
        }
        let config_path = root_path.join("launch.json");
        let encoded = serde_json::to_vec(&config).map_err(|_| ModelError::Request)?;
        if encoded.len() > 16384 {
            return Err(ModelError::Request);
        }
        let config_sha = format!("{:x}", Sha256::digest(&encoded));
        write_new(&config_path, &encoded)?;
        let (config_path, config_pin) = pin(&config_path, &config_sha)?;
        let mut pins = self.pins;
        pins.push(sdk_pin);
        pins.push(config_pin);
        pins.push(selected.pin);
        if let Some(file) = template_pin {
            pins.push(file);
        }
        let root_pin = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x0200_0000)
            .open(&root_path)
            .map_err(|_| ModelError::Request)?;
        pins.push(root_pin);
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let arguments = vec![
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
            config_sha,
            nonce.clone(),
        ];
        let mut paths = self.paths;
        paths.push(selected.path);
        let profile = SandboxLaunchProfile::new(
            &root_path,
            std::slice::from_ref(&root_path),
            &[],
            vec![],
            SandboxNetworkPolicy::Disabled,
            SandboxResourceLimits {
                max_processes: 2,
                max_memory_bytes: selected.record.runtime_memory_bytes,
                max_cpu_time_ms: 300_000,
            },
        )
        .map_err(|_| ModelError::Request)?
        .with_cpu_semantics(gent_types::SandboxCpuSemantics::Uncapped)
        .map_err(|_| ModelError::Request)?
        .with_permission_binding(run.binding.clone())
        .with_resources(
            paths
                .into_iter()
                .map(SandboxResourceGrant::RuntimeReadOnly)
                .collect(),
        )
        .map_err(|_| ModelError::Request)?;
        let request = SandboxedLaunchRequest {
            lock: self.worker.clone(),
            profile,
        };
        let launch = ProviderLaunch {
            permission_binding: Some(run.binding.clone()),
            run_id: Some(run.binding.run_id().into()),
            lock: self.worker.clone(),
            provider: self.worker.provider.clone(),
            executable: self.worker.canonical_path.clone().into(),
            arguments,
            intent: LaunchIntent::Start,
            workspace_root: Some(root_path.clone()),
            workspace_access: SandboxWorkspaceAccess::ReadOnly,
        };
        let resources = ModelResources {
            pins,
            root,
            lifetime: run.lifetime,
        };
        let backend = gent_drivers::windows_sandbox::WindowsSandboxLauncher::new(
            self.helper,
            8 * 1024 * 1024,
        );
        let Ok(process) = backend.launch_sandboxed(&request, &launch) else {
            scope::retain_launch_error(resources);
            return Err(ModelError::Transport);
        };
        let process = Arc::new(process);
        let mut endpoint = ModelEndpoint::start(
            super::super::Scope::new(
                process,
                request,
                run.binding.run_id().into(),
                selected.record.sha256,
                selected.record.provider_model_id,
                run.current,
                resources,
            ),
            nonce,
        )?;
        endpoint.sdk_lock = Some(run.sdk_lock);
        Ok(endpoint)
    }
}

fn admissible(run: &ModelRun, selected: &SelectedModel, config: &serde_json::Value) -> bool {
    !(run.sdk_lock.provider != "claurst"
        || gent_drivers::lock::rechecked_identity(&run.sdk_lock)
            .ok()
            .as_ref()
            != Some(&run.sdk_lock)
        || run.writable_roots.is_empty()
        || config.get("templatePath").is_some()
        || config.get("templateSha256").is_some()
        || !(run.current)()
        || config["modelSha256"].as_str() != Some(&selected.record.sha256)
        || config["modelId"].as_str() != Some(&selected.record.provider_model_id)
        || config["modelPath"]
            .as_str()
            .and_then(|path| Path::new(path).canonicalize().ok())
            .as_ref()
            != Some(&selected.path)
        || config["contextTokens"].as_u64() != Some(u64::from(selected.record.context_tokens)))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ModelError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| ModelError::Request)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| ModelError::Transport)
}
