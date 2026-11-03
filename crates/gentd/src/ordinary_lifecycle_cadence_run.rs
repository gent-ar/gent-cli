use std::sync::Arc;

use super::{ACTIVE_POLL_INTERVAL, OrdinaryLifecycleCadence};

impl<L: Send + 'static> OrdinaryLifecycleCadence<L> {
    pub(crate) async fn run(self) -> Result<(), String> {
        if self.control.phase()
            == crate::ordinary_lifecycle_control::OrdinaryLifecyclePhase::Draining
        {
            return Ok(());
        }
        self.activate_recovery().await?;
        self.control.open_after_recovery();
        self.drive_until_idle(false).await?;
        if self.control.phase()
            == crate::ordinary_lifecycle_control::OrdinaryLifecyclePhase::Draining
        {
            return self.drain_shutdown().await;
        }
        loop {
            tokio::select! {
                () = self.control.shutdown_requested() => return self.drain_shutdown().await,
                () = self.notify.notified() => self.drive_until_idle(false).await?,
                () = self.async_claurst_notify.notified() => self.drive_until_idle(false).await?,
            }
        }
    }

    async fn activate_recovery(&self) -> Result<(), String> {
        let router = Arc::clone(&self.router);
        tokio::task::spawn_blocking(move || {
            router
                .lock()
                .map_err(|_| "ordinary lifecycle router is unavailable".to_owned())?
                .activate_recovery()
                .map_err(|_| "ordinary lifecycle recovery was rejected".to_owned())
        })
        .await
        .map_err(|_| "ordinary lifecycle recovery task failed".to_owned())??;
        if let Some(host) = self.async_claurst.lock().await.as_mut() {
            host.activate_recovery().await?;
        }
        if let Some(recovery) = &self.standalone_recovery {
            recovery.recover().await?;
        }
        Ok(())
    }

    async fn drive_until_idle(&self, shutting_down: bool) -> Result<(), String> {
        loop {
            if !shutting_down && self.control.is_quiesced() {
                return Ok(());
            }
            let router = Arc::clone(&self.router);
            let active = tokio::task::spawn_blocking(move || {
                router
                    .lock()
                    .map_err(|_| "ordinary lifecycle router is unavailable".to_owned())?
                    .drive_once()
                    .map_err(|_| "ordinary lifecycle drive was rejected".to_owned())
            })
            .await
            .map_err(|_| "ordinary lifecycle drive task failed".to_owned())??;
            let claurst_active = if let Some(host) = self.async_claurst.lock().await.as_mut() {
                if !shutting_down && self.control.is_quiesced() {
                    return Ok(());
                }
                let interrupts = self
                    .async_claurst_interrupts
                    .lock()
                    .map_err(|_| "Claurst interrupt queue is unavailable".to_owned())?
                    .drain(..)
                    .collect::<Vec<_>>();
                for interrupt in interrupts {
                    let result = match &interrupt.expected_turn_id {
                        Some(turn_id) => {
                            host.interrupt_claurst_turn(&interrupt.run_id, turn_id)
                                .await
                        }
                        None => host.interrupt_claurst_run(&interrupt.run_id).await,
                    };
                    match result {
                        Ok(()) => {
                            if let Some(stop) = interrupt.goal_stop {
                                stop.apply();
                            }
                        }
                        Err(error) => eprintln!(
                            "Claurst interrupt for run {} was not applied: {error}",
                            interrupt.run_id
                        ),
                    }
                }
                host.drive_once().await?
            } else {
                false
            };
            if !active && !claurst_active {
                return Ok(());
            }
            tokio::time::sleep(ACTIVE_POLL_INTERVAL).await;
        }
    }

    async fn drain_shutdown(&self) -> Result<(), String> {
        self.control.wait_for_permits().await;
        let router = Arc::clone(&self.router);
        tokio::task::spawn_blocking(move || {
            router
                .lock()
                .map_err(|_| "ordinary lifecycle router is unavailable".to_owned())?
                .begin_shutdown_after_recovery()
                .map_err(|_| "ordinary lifecycle shutdown was rejected".to_owned())
        })
        .await
        .map_err(|_| "ordinary lifecycle shutdown task failed".to_owned())??;
        if let Some(host) = self.async_claurst.lock().await.as_mut() {
            host.begin_shutdown_after_recovery().await?;
        }
        self.drive_until_idle(true).await?;
        let router = Arc::clone(&self.router);
        let stopped = tokio::task::spawn_blocking(move || {
            router
                .lock()
                .map_err(|_| "ordinary lifecycle router is unavailable".to_owned())
                .map(|router| router.shutdown_complete())
        })
        .await
        .map_err(|_| "ordinary lifecycle shutdown task failed".to_owned())??;
        let claurst_stopped = self
            .async_claurst
            .lock()
            .await
            .as_ref()
            .is_none_or(|host| host.shutdown_complete());
        (stopped && claurst_stopped)
            .then_some(())
            .ok_or_else(|| "ordinary lifecycle shutdown was not proven by its owner".to_owned())
    }
}
