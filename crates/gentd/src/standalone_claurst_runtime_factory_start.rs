use super::{
    ActiveRuntime, ClaurstBridgeHandle, ClaurstStandaloneOwner, HttpLlamaServerReadiness, PathBuf,
    RuntimeIdentity, Session, StandaloneClaurstRuntimeFactory, SystemPrivateSettingsStore,
};
use crate::claurst_runtime_factory::CLAURST_CONVERSATION_CAPACITY;
use crate::standalone_claurst_runtime_factory::SystemBridge;
use crate::standalone_claurst_runtime_identity::RuntimeReuse;

type SystemModel = crate::claurst_standalone_owner::ClaurstStandaloneModel<
    crate::claurst_sandbox_launcher::model_launch::RoutedModelProcess,
>;

impl StandaloneClaurstRuntimeFactory {
    pub(super) async fn reuse_ready(
        &self,
        conversation_id: &str,
        run_id: &str,
        identity: &RuntimeIdentity,
    ) -> bool {
        let mut active = self.active.lock().await;
        let Some(running) = active.get_mut(conversation_id) else {
            return false;
        };
        let ready = matches!(running.runtime.exited(), Ok(None))
            && identity.reuse_from(&running.identity) == RuntimeReuse::Ready;
        if ready {
            run_id.clone_into(&mut running.run_id);
            running.used = std::time::Instant::now();
        }
        ready
    }

    pub(super) async fn launch_session(
        &self,
        session: Session<'_>,
        request: crate::claurst_local_runtime::ClaurstLocalRuntimeRequest,
        workspace: PathBuf,
        mcp_servers: Vec<serde_json::Value>,
        busy: &[String],
    ) -> Result<(), String> {
        let (previous, model) = {
            let mut active = self.active.lock().await;
            let mut previous = active.remove(&session.conversation_id);
            let model = previous
                .iter_mut()
                .chain(active.values_mut())
                .find_map(|running| shared_model(&session.identity, running));
            (previous, model)
        };
        if let Some(previous) = previous {
            previous
                .shut_down("could not stop previous local Claurst runtime")
                .await?;
        }
        self.evict_idle(&session.conversation_id, busy).await?;
        let owner = ClaurstStandaloneOwner::new(
            self.readiness.clone(),
            SystemPrivateSettingsStore,
            crate::claurst_sandbox_launcher::RoutedClaurstStandaloneLauncher::new(
                self.ledger.clone(),
                session.identity.permission_binding().cloned(),
            )?
            .with_mcp_control(self.mcp_control.clone()),
            HttpLlamaServerReadiness::default(),
        );
        let model_id = session.identity.model.model_id.clone();
        let runtime = tokio::task::spawn_blocking(move || {
            model
                .and_then(|model| {
                    owner
                        .attach_session(
                            &model,
                            &model_id,
                            request.clone(),
                            &workspace,
                            mcp_servers.clone(),
                        )
                        .ok()
                })
                .map_or_else(
                    || owner.start_with_mcp(&model_id, request, &workspace, mcp_servers),
                    Ok,
                )
        })
        .await
        .map_err(|_| "local Claurst startup worker stopped unexpectedly".to_owned())?
        .map_err(|error| error.to_string())?;
        let started = ActiveRuntime {
            identity: session.identity,
            run_id: session.run_id.to_owned(),
            used: std::time::Instant::now(),
            bridge: ClaurstBridgeHandle::new(runtime.bridge()),
            runtime,
        };
        if let Err(error) = started.bridge.initialize().await {
            started
                .shut_down("could not stop unready local Claurst runtime")
                .await?;
            return Err(error.to_string());
        }
        let rejected = match self.active.lock().await.entry(session.conversation_id) {
            std::collections::btree_map::Entry::Occupied(_) => Some(started),
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(started);
                None
            }
        };
        match rejected {
            Some(started) => {
                started
                    .shut_down("could not stop duplicate local Claurst runtime")
                    .await?;
                Err("local Claurst runtime changed while it was starting".into())
            }
            None => Ok(()),
        }
    }

    async fn evict_idle(&self, conversation_id: &str, busy: &[String]) -> Result<(), String> {
        let evicted = {
            let mut active = self.active.lock().await;
            let mut evicted = Vec::new();
            while active.len() >= CLAURST_CONVERSATION_CAPACITY {
                let sessions = active.iter().map(|(conversation, running)| {
                    let idle = running.bridge.is_idle().unwrap_or(false);
                    (conversation, idle, running.used)
                });
                let Some(idle) = idle_victim(sessions, conversation_id, busy) else {
                    break;
                };
                evicted.extend(active.remove(&idle));
            }
            evicted
        };
        for idle in evicted {
            idle.shut_down("could not stop idle local Claurst runtime")
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn run_bridge(
        &self,
        run_id: &str,
    ) -> Result<SystemBridge, gent_ports::PortError> {
        self.active
            .lock()
            .await
            .values()
            .find(|active| active.run_id == run_id)
            .map(|active| active.bridge.clone())
            .ok_or_else(|| {
                gent_ports::PortError::Unavailable("local Claurst runtime is not ready".into())
            })
    }

    pub(crate) async fn summary_bridge(
        &self,
        model_id: &str,
    ) -> Result<SystemBridge, gent_ports::PortError> {
        let active = self.active.lock().await;
        let mut loaded = active
            .values()
            .filter(|runtime| runtime.identity.model.model_id == model_id)
            .peekable();
        if loaded.peek().is_none() {
            return Err(gent_ports::PortError::Unavailable(
                "selected Claurst model is not an active local model".into(),
            ));
        }
        loaded
            .find(|runtime| runtime.bridge.is_idle().unwrap_or(false))
            .map(|runtime| runtime.bridge.clone())
            .ok_or_else(|| {
                gent_ports::PortError::Unavailable(
                    "Claurst summary waits for an interactive prompt to become idle".into(),
                )
            })
    }

    pub(crate) async fn stop_conversation(&self, conversation_id: &str) -> Result<(), String> {
        let previous = self.active.lock().await.remove(conversation_id);
        if let Some(previous) = previous {
            previous
                .shut_down("could not stop local Claurst runtime")
                .await?;
        }
        Ok(())
    }

    pub(crate) fn has_active_runtime(&self) -> bool {
        self.active
            .try_lock()
            .map_or(true, |active| !active.is_empty())
    }
}

fn idle_victim<'a>(
    sessions: impl Iterator<Item = (&'a String, bool, std::time::Instant)>,
    starting: &str,
    busy: &[String],
) -> Option<String> {
    sessions
        .filter(|(conversation, idle, _)| {
            *idle && conversation.as_str() != starting && !busy.contains(conversation)
        })
        .min_by_key(|(_, _, used)| *used)
        .map(|(conversation, _, _)| conversation.clone())
}

fn shared_model(identity: &RuntimeIdentity, running: &mut ActiveRuntime) -> Option<SystemModel> {
    (matches!(running.runtime.exited(), Ok(None))
        && identity.reuse_from(&running.identity) != RuntimeReuse::Relaunch)
        .then(|| running.runtime.model())
}

#[cfg(test)]
#[path = "standalone_claurst_runtime_factory_eviction_tests.rs"]
mod eviction_tests;
