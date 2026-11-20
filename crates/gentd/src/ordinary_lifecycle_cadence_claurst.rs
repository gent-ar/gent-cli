use std::sync::atomic::Ordering;

use super::{AsyncOrdinaryLifecycleHost, OrdinaryPromptIngress};

impl<L> OrdinaryPromptIngress<L> {
    pub(crate) async fn attach_async_claurst(
        &self,
        host: Box<dyn AsyncOrdinaryLifecycleHost>,
    ) -> Result<(), String> {
        let mut slot = self.async_claurst.lock().await;
        if slot.is_some() {
            return Err("standalone Claurst owner is already attached".into());
        }
        *slot = Some(host);
        self.async_claurst_attached.store(true, Ordering::Release);
        Ok(())
    }

    pub(crate) async fn respond_claurst_permission(
        &self,
        response: gent_types::PermissionDecisionResponse,
    ) -> Result<(), String> {
        let answered = self
            .async_claurst
            .lock()
            .await
            .as_mut()
            .ok_or_else(|| "Claurst permission owner is unavailable".to_owned())?
            .respond_claurst_permission(response)
            .await;
        self.wake_claurst();
        answered
    }

    fn wake_claurst(&self) {
        if let Some(notify) = &self.async_claurst_notify {
            notify.notify_one();
        }
    }

    pub(super) fn interrupt_claurst(
        &self,
        run_id: &str,
        turn_id: Option<String>,
        goal_stop: Option<super::InterruptGoalStop>,
    ) -> Result<(), String> {
        if !self.async_claurst_attached.load(Ordering::Acquire) {
            return Err("Claurst interrupt owner is unavailable".into());
        }
        let host = std::sync::Arc::clone(&self.async_claurst);
        let notify = self.async_claurst_notify.clone();
        let run_id = run_id.to_owned();
        tokio::spawn(async move {
            let result = match host.lock().await.as_mut() {
                None => Err("Claurst interrupt owner is unavailable".to_owned()),
                Some(host) => match &turn_id {
                    Some(turn_id) => host.interrupt_claurst_turn(&run_id, turn_id).await,
                    None => host.interrupt_claurst_run(&run_id).await,
                },
            };
            match result {
                Ok(()) => {
                    if let Some(stop) = goal_stop {
                        stop.apply();
                    }
                }
                Err(error) => {
                    eprintln!("Claurst interrupt for run {run_id} was not applied: {error}");
                }
            }
            if let Some(notify) = notify {
                notify.notify_one();
            }
        });
        Ok(())
    }

    pub(crate) async fn respond_claurst_permission_with_receipt(
        &self,
        response: gent_types::PermissionDecisionResponse,
        receipt_id: gent_types::ReceiptId,
    ) -> Result<gent_types::Receipt, String> {
        let answered = self
            .async_claurst
            .lock()
            .await
            .as_mut()
            .ok_or_else(|| "Claurst permission owner is unavailable".to_owned())?
            .respond_claurst_permission_with_receipt(response, receipt_id)
            .await;
        self.wake_claurst();
        answered
    }
}
