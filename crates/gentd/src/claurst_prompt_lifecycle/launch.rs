use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use gent_ports::{ClaurstPromptAttachment, ClaurstSourceId, PrivateClaurstBridge};
use gent_types::AgentChatPromptSaved;
use tokio::sync::oneshot::{self, error::TryRecvError};
use tokio::task::JoinHandle;

use super::compaction::Work;
use super::{ActivePrompt, ClaurstLifecycleLedger, ClaurstPromptLifecycle};
use crate::claurst_runtime_factory::ClaurstRuntimeFactory;

#[derive(Debug)]
pub(super) struct Launch {
    pub(super) attachments: Vec<ClaurstPromptAttachment>,
    pub(super) without_attachments: bool,
}

#[derive(Debug)]
pub(super) struct Launching {
    launch: Launch,
    runtime: oneshot::Receiver<Result<(), String>>,
}

struct Delivery<T>(Option<oneshot::Sender<T>>);

impl<T> Drop for Delivery<T> {
    fn drop(&mut self) {
        self.0.take();
        gent_drivers::provider_activity::signal();
    }
}

pub(super) fn signalled<T: Send + 'static>(
    work: impl Future<Output = T> + Send + 'static,
) -> (JoinHandle<()>, oneshot::Receiver<T>) {
    let (sender, receiver) = oneshot::channel();
    let delivery = Delivery(Some(sender));
    let task = tokio::spawn(async move {
        let mut delivery = delivery;
        let output = work.await;
        if let Some(sender) = delivery.0.take() {
            let _ = sender.send(output);
        }
    });
    (task, receiver)
}

impl<L, B, F> ClaurstPromptLifecycle<L, B, F>
where
    L: ClaurstLifecycleLedger,
    B: PrivateClaurstBridge,
    F: ClaurstRuntimeFactory,
{
    pub(super) async fn launch_runtime(
        &mut self,
        saved: AgentChatPromptSaved,
        source_id: ClaurstSourceId,
        launch: Launch,
    ) -> Result<bool, String> {
        let runtime = Arc::clone(&self.runtime);
        let busy = self.busy_conversations();
        let prompt = saved.clone();
        let mut ensure = Box::pin(async move { runtime.ensure_for_prompt(&prompt, &busy).await });
        let ready = ensure
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()));
        if let Poll::Ready(launched) = ready {
            return self
                .start_launched(saved, source_id, launch, launched)
                .await;
        }
        let (_, runtime) = signalled(ensure);
        self.active.insert(
            source_id,
            ActivePrompt {
                saved,
                work: Work::Launching(Launching { launch, runtime }),
            },
        );
        Ok(true)
    }

    pub(super) async fn drain_launch(
        &mut self,
        source_id: &ClaurstSourceId,
    ) -> Result<bool, String> {
        let received = match self
            .active
            .get_mut(source_id)
            .map(|active| &mut active.work)
        {
            Some(Work::Launching(launching)) => launching.runtime.try_recv(),
            _ => return Ok(false),
        };
        if matches!(received, Err(TryRecvError::Empty)) {
            return Ok(false);
        }
        let Some(ActivePrompt {
            saved,
            work: Work::Launching(Launching { launch, .. }),
        }) = self.active.remove(source_id)
        else {
            return Err("finished Claurst launch is no longer active".into());
        };
        let launched = received
            .unwrap_or_else(|_| Err("local Claurst startup worker stopped unexpectedly".into()));
        self.start_launched(saved, source_id.clone(), launch, launched)
            .await
    }
}
