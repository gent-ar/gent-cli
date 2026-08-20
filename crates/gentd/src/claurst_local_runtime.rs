//! Pure launch planning for a Gent-owned local llama.cpp Claurst ACP runtime.
//!
//! The effect owner creates the returned settings file, starts `llama-server`, waits for
//! its local readiness endpoint, and only then starts `claurst acp`.  Keeping that process
//! orchestration out of this module makes the selected model and upstream configuration
//! inspectable before any executable is started.

use std::{collections::BTreeMap, path::PathBuf};

use serde_json::json;

use crate::local_model_catalog::LocalModelRecord;

const LLAMA_CPP_PROVIDER: &str = "llamacpp";
const DEFAULT_CONTEXT_SIZE: u32 = 32_768;

/// Inputs owned by the future local-runtime lifecycle host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaurstLocalRuntimeRequest {
    pub(crate) claurst_executable: PathBuf,
    pub(crate) llama_server_executable: PathBuf,
    /// Exact ready GGUF path obtained from `LocalModelProvisioner`.
    pub(crate) model_path: PathBuf,
    pub(crate) claurst_home: PathBuf,
    pub(crate) port: u16,
}

/// One concrete child-process invocation without process ownership.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalProcessLaunch {
    pub(crate) executable: PathBuf,
    pub(crate) arguments: Vec<String>,
    pub(crate) environment: BTreeMap<String, String>,
}

/// All durable inputs an effect owner needs to start Claurst through local llama.cpp.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaurstLocalRuntimePlan {
    pub(crate) model_path: PathBuf,
    pub(crate) server_url: String,
    pub(crate) settings_path: PathBuf,
    pub(crate) settings_json: String,
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
    /// Produces the isolated settings and exact upstream process commands for one curated model.
    ///
    /// `llama-server` receives the model's embedded Jinja template and one full-context slot;
    /// Claurst receives its documented `LLAMA_CPP_HOST` and is started in ACP stdio mode.
    pub(crate) fn build(
        request: ClaurstLocalRuntimeRequest,
        model: &LocalModelRecord,
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
        if request.port == 0 {
            return Err(ClaurstLocalRuntimePlanError::ZeroPort);
        }

        let model_path = request.model_path;
        let server_url = format!("http://127.0.0.1:{}", request.port);
        let settings_path = request.claurst_home.join("settings.json");
        // `claurst acp` has no provider/model CLI flags: its runtime loads this settings file.
        let settings_json = serde_json::to_string_pretty(&json!({
            "provider": LLAMA_CPP_PROVIDER,
            "config": {
                "provider": LLAMA_CPP_PROVIDER,
                "model": format!("{LLAMA_CPP_PROVIDER}/{}", model.provider_model_id),
            },
        }))
        .expect("a fixed JSON settings document serializes");

        let llama_server = LocalProcessLaunch {
            executable: request.llama_server_executable,
            arguments: vec![
                "-m".into(),
                model_path.display().to_string(),
                "--host".into(),
                "127.0.0.1".into(),
                "--port".into(),
                request.port.to_string(),
                "--jinja".into(),
                "--ctx-size".into(),
                DEFAULT_CONTEXT_SIZE.to_string(),
                "--parallel".into(),
                "1".into(),
            ],
            environment: BTreeMap::new(),
        };
        let claurst_acp = LocalProcessLaunch {
            executable: request.claurst_executable,
            arguments: vec!["acp".into()],
            environment: BTreeMap::from([
                (
                    "CLAURST_HOME".into(),
                    request.claurst_home.display().to_string(),
                ),
                ("LLAMA_CPP_HOST".into(), server_url.clone()),
            ]),
        };

        Ok(Self {
            model_path,
            server_url,
            settings_path,
            settings_json,
            llama_server,
            claurst_acp,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        ClaurstLocalRuntimePlan, ClaurstLocalRuntimePlanError, ClaurstLocalRuntimeRequest,
    };
    use crate::local_model_catalog::LocalModelCatalog;

    fn request() -> ClaurstLocalRuntimeRequest {
        ClaurstLocalRuntimeRequest {
            claurst_executable: PathBuf::from("/opt/gent/bin/claurst"),
            llama_server_executable: PathBuf::from("/opt/gent/bin/llama-server"),
            model_path: PathBuf::from("/opt/gent/models/qwen3-4b-q4-k-m/qwen3-4b-q4-k-m.gguf"),
            claurst_home: PathBuf::from("/opt/gent/claurst"),
            port: 18_080,
        }
    }

    #[test]
    fn builds_an_isolated_llama_cpp_acp_plan_for_the_curated_model() {
        let catalog = LocalModelCatalog::shipped().unwrap();
        let plan =
            ClaurstLocalRuntimePlan::build(request(), catalog.model("qwen3-4b-q4-k-m").unwrap())
                .unwrap();

        assert_eq!(
            plan.model_path,
            PathBuf::from("/opt/gent/models/qwen3-4b-q4-k-m/qwen3-4b-q4-k-m.gguf")
        );
        assert_eq!(plan.server_url, "http://127.0.0.1:18080");
        assert_eq!(
            plan.settings_path,
            PathBuf::from("/opt/gent/claurst/settings.json")
        );
        assert!(plan.settings_json.contains("\"provider\": \"llamacpp\""));
        assert!(plan.settings_json.contains("llamacpp/qwen3-4b-q4-k-m"));
        assert_eq!(
            plan.llama_server.arguments,
            vec![
                "-m",
                "/opt/gent/models/qwen3-4b-q4-k-m/qwen3-4b-q4-k-m.gguf",
                "--host",
                "127.0.0.1",
                "--port",
                "18080",
                "--jinja",
                "--ctx-size",
                "32768",
                "--parallel",
                "1",
            ]
        );
        assert_eq!(plan.claurst_acp.arguments, ["acp"]);
        assert_eq!(
            plan.claurst_acp.environment.get("LLAMA_CPP_HOST"),
            Some(&"http://127.0.0.1:18080".to_string())
        );
        assert_eq!(
            plan.claurst_acp.environment.get("CLAURST_HOME"),
            Some(&"/opt/gent/claurst".to_string())
        );
    }

    #[test]
    fn rejects_relative_paths_and_invalid_port_before_any_effect() {
        let catalog = LocalModelCatalog::shipped().unwrap();
        let mut invalid = request();
        invalid.model_path = PathBuf::from("models");
        assert_eq!(
            ClaurstLocalRuntimePlan::build(invalid, catalog.models().first().unwrap()),
            Err(ClaurstLocalRuntimePlanError::RelativePath("model file"))
        );
        let mut invalid = request();
        invalid.port = 0;
        assert_eq!(
            ClaurstLocalRuntimePlan::build(invalid, catalog.models().first().unwrap()),
            Err(ClaurstLocalRuntimePlanError::ZeroPort)
        );
    }
}
