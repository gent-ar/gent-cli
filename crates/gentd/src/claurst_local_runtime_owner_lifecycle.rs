use crate::claurst_local_runtime::ClaurstLocalRuntimePlan;

use super::{
    LlamaServerReadiness, LocalRuntimeLauncher, LocalRuntimeProcess, PrivateSettingsStore,
};

struct ActiveRuntime<P> {
    llama_server: P,
    claurst_acp: P,
}

pub(crate) struct ClaurstLocalRuntimeOwner<S, L: LocalRuntimeLauncher, R> {
    settings: S,
    launcher: L,
    readiness: R,
    active: Option<ActiveRuntime<L::Process>>,
}

impl<S, L, R> ClaurstLocalRuntimeOwner<S, L, R>
where
    S: PrivateSettingsStore,
    L: LocalRuntimeLauncher,
    R: LlamaServerReadiness,
{
    #[must_use]
    pub(crate) fn new(settings: S, launcher: L, readiness: R) -> Self {
        Self {
            settings,
            launcher,
            readiness,
            active: None,
        }
    }

    pub(crate) fn start(&mut self, plan: &ClaurstLocalRuntimePlan) -> Result<(), String> {
        if self.active.is_some() {
            return Err("a local Claurst runtime is already active".into());
        }
        self.settings
            .materialize(&plan.settings_path, &plan.settings_json)
            .map_err(|error| format!("could not materialize private Claurst settings: {error}"))?;
        if let (Some(path), Some(contents)) =
            (&plan.chat_template_path, &plan.chat_template_contents)
        {
            self.settings.materialize(path, contents).map_err(|error| {
                format!("could not materialize the local tool template: {error}")
            })?;
        }
        let mut llama_server = self
            .launcher
            .launch(&plan.llama_server)
            .map_err(|error| format!("could not start llama.cpp server: {error}"))?;
        if let Err(error) = self
            .readiness
            .wait_ready(&plan.server_url, &mut llama_server)
        {
            let _ = llama_server.shutdown();
            return Err(format!("local llama.cpp server was not ready: {error}"));
        }
        let claurst_acp = match self.launcher.launch(&plan.claurst_acp) {
            Ok(process) => process,
            Err(error) => {
                let _ = llama_server.shutdown();
                return Err(format!("could not start Claurst ACP: {error}"));
            }
        };
        self.active = Some(ActiveRuntime {
            llama_server,
            claurst_acp,
        });
        Ok(())
    }

    pub(crate) fn shutdown(&mut self) -> Result<(), String> {
        let Some(mut active) = self.active.take() else {
            return Ok(());
        };
        let acp = active.claurst_acp.shutdown();
        let llama = active.llama_server.shutdown();
        match (acp, llama) {
            (Ok(()), Ok(())) => Ok(()),
            (acp, llama) => {
                self.active = Some(active);
                let acp_error = acp.err().unwrap_or_else(|| "ok".to_owned());
                let llama_error = llama.err().unwrap_or_else(|| "ok".to_owned());
                Err(format!(
                    "local Claurst runtime shutdown failed (acp: {acp_error}; llama.cpp: {llama_error})"
                ))
            }
        }
    }
}
