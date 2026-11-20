use super::{
    ActiveRuntime, ClaurstBridgeHandle, ClaurstStandaloneOwner, HttpLlamaServerReadiness, PathBuf,
    RuntimeIdentity, Session, StandaloneClaurstRuntimeFactory, SystemPrivateSettingsStore,
};
use crate::claurst_runtime_factory::{CLAURST_CONVERSATION_CAPACITY, LlamaContextSummarizer};
use crate::standalone_claurst_runtime_factory::SystemBridge;
use crate::standalone_claurst_runtime_identity::RuntimeReuse;
use std::sync::Arc;

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
            self.end_background(&session.conversation_id, gent_types::ToolPhase::Failed)?;
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
            idle_since: None,
            bridge: ClaurstBridgeHandle::new(runtime.bridge()),
            runtime,
        };
        started
            .bridge
            .bind_owned_chat_tools(session.owned_chat_tools)
            .map_err(|error| error.to_string())?;
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

    pub(crate) async fn reap_idle_runtimes(
        &self,
        now: std::time::Instant,
        busy: &[String],
    ) -> Result<(), String> {
        let expired = {
            let mut active = self.active.lock().await;
            let expired = idle_expired(
                active.iter_mut().map(|(conversation, running)| {
                    let idle = running.bridge.is_idle().unwrap_or(false);
                    (conversation, idle, &mut running.idle_since)
                }),
                busy,
                now,
            );
            expired
                .iter()
                .filter_map(|conversation| active.remove(conversation))
                .collect::<Vec<_>>()
        };
        let mut failure = None;
        for idle in expired {
            if let Err(error) = idle
                .shut_down("could not stop idle local Claurst runtime")
                .await
            {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
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

    pub(crate) async fn side_question_model(
        &self,
        conversation_id: &str,
        model_id: &str,
    ) -> Result<Arc<LlamaContextSummarizer>, gent_ports::PortError> {
        if let Some(model) = self.loaded_model(conversation_id, model_id).await {
            return Ok(model);
        }
        let run_id = gent_runtime::AgentChatReadService::new(self.ledger.clone())
            .detail(conversation_id)
            .map_err(|error| gent_ports::PortError::Unavailable(error.to_string()))?
            .current_run_id;
        self.start_selected(conversation_id, &run_id, &[], &[])
            .await
            .map_err(gent_ports::PortError::Unavailable)?;
        self.loaded_model(conversation_id, model_id)
            .await
            .ok_or_else(|| {
                gent_ports::PortError::Unavailable(
                    "the conversation's local model is not the one it selected".into(),
                )
            })
    }

    async fn loaded_model(
        &self,
        conversation_id: &str,
        model_id: &str,
    ) -> Option<Arc<LlamaContextSummarizer>> {
        let mut active = self.active.lock().await;
        let running = active.get_mut(conversation_id)?;
        if running.identity.model.model_id != model_id
            || !matches!(running.runtime.exited(), Ok(None))
        {
            return None;
        }
        running.used = std::time::Instant::now();
        running.idle_since = None;
        Some(running.runtime.summarizer())
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

fn idle_expired<'a>(
    sessions: impl Iterator<Item = (&'a String, bool, &'a mut Option<std::time::Instant>)>,
    busy: &[String],
    now: std::time::Instant,
) -> Vec<String> {
    let idle = sessions
        .filter_map(|(conversation, idle, since)| {
            if !idle || busy.contains(conversation) {
                *since = None;
                return None;
            }
            Some((conversation, *since.get_or_insert(now)))
        })
        .collect::<Vec<_>>();
    crate::provider_warm_pool::expired(idle.into_iter().map(|(conversation, since)| {
        crate::provider_warm_pool::IdleRun {
            run_id: conversation,
            idle_for: now.saturating_duration_since(since),
            served: true,
            current: true,
        }
    }))
}

fn shared_model(identity: &RuntimeIdentity, running: &mut ActiveRuntime) -> Option<SystemModel> {
    (matches!(running.runtime.exited(), Ok(None))
        && identity.reuse_from(&running.identity) != RuntimeReuse::Relaunch)
        .then(|| running.runtime.model())
}

#[cfg(test)]
#[path = "standalone_claurst_runtime_factory_eviction_tests.rs"]
mod eviction_tests;
