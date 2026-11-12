use std::sync::Arc;

use super::{ACTIVE_POLL_INTERVAL, OrdinaryLifecycleCadence};

const WARM_POOL_REAP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

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
        let mut reap = tokio::time::interval(WARM_POOL_REAP_INTERVAL);
        reap.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = self.control.shutdown_requested() => return self.drain_shutdown().await,
                () = self.notify.notified() => self.drive_until_idle(false).await?,
                () = self.async_claurst_notify.notified() => self.drive_until_idle(false).await?,
                _ = reap.tick() => self.reap_warm_pool().await,
            }
        }
    }

    async fn reap_warm_pool(&self) {
        let router = Arc::clone(&self.router);
        let reaped = tokio::task::spawn_blocking(move || {
            router
                .lock()
                .map_err(|_| "ordinary lifecycle router is unavailable".to_owned())?
                .tend_warm_pool(
                    None,
                    &crate::provider_warm_pool::WarmPoolRequest {
                        now: std::time::Instant::now(),
                        target: None,
                    },
                )
                .map(|_| ())
                .map_err(|_| "ordinary lifecycle warm pool was rejected".to_owned())
        })
        .await
        .map_err(|_| "ordinary lifecycle warm pool task failed".to_owned())
        .and_then(|reaped| reaped);
        if let Err(error) = reaped {
            eprintln!("{error}");
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
        let mut wake_idle_hosts = false;
        let mut claurst_working = false;
        loop {
            if !shutting_down && self.control.is_quiesced() {
                return Ok(());
            }
            let router = Arc::clone(&self.router);
            let (active, router_finished) = tokio::task::spawn_blocking(move || {
                let mut router = router
                    .lock()
                    .map_err(|_| "ordinary lifecycle router is unavailable".to_owned())?;
                if wake_idle_hosts {
                    router
                        .wake_idle_hosts()
                        .map_err(|_| "ordinary lifecycle wake was rejected".to_owned())?;
                }
                let active = router
                    .drive_once()
                    .map_err(|_| "ordinary lifecycle drive was rejected".to_owned())?;
                Ok::<_, String>((active, router.finished_work()))
            })
            .await
            .map_err(|_| "ordinary lifecycle drive task failed".to_owned())??;
            let (claurst_active, claurst_finished) =
                match Arc::clone(&self.async_claurst).try_lock_owned() {
                    Err(_) => (true, false),
                    Ok(host) => match self.drive_claurst(host, shutting_down).await? {
                        Some(active) => (
                            active,
                            std::mem::replace(&mut claurst_working, active) && !active,
                        ),
                        None => return Ok(()),
                    },
                };
            wake_idle_hosts = !shutting_down && (router_finished || claurst_finished);
            if !active && !claurst_active && !wake_idle_hosts {
                return Ok(());
            }
            tokio::time::sleep(ACTIVE_POLL_INTERVAL).await;
        }
    }

    async fn drive_claurst(
        &self,
        mut guard: tokio::sync::OwnedMutexGuard<Option<Box<dyn super::AsyncOrdinaryLifecycleHost>>>,
        shutting_down: bool,
    ) -> Result<Option<bool>, String> {
        let Some(host) = guard.as_mut() else {
            return Ok(Some(false));
        };
        if !shutting_down && self.control.is_quiesced() {
            return Ok(None);
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
        let active = host.drive_once().await?;
        let warm = self
            .async_claurst_warm
            .lock()
            .map_err(|_| "Claurst warm-up slot is unavailable".to_owned())?
            .take();
        if let Some(target) = warm.filter(|_| !active) {
            let notify = Arc::clone(&self.async_claurst_notify);
            tokio::spawn(async move {
                if let Some(host) = guard.as_mut()
                    && let Err(error) = host
                        .warm_claurst(&target.conversation_id, &target.run_id)
                        .await
                {
                    eprintln!(
                        "Claurst warm-up for run {} was unavailable: {error}",
                        target.run_id
                    );
                }
                drop(guard);
                notify.notify_one();
            });
        }
        Ok(Some(active))
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
