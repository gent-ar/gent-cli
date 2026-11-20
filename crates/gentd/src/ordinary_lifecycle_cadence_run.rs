use std::sync::{Arc, atomic::Ordering};

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
        tokio::try_join!(self.run_router(), self.run_claurst()).map(|_| ())
    }

    async fn run_router(&self) -> Result<(), String> {
        let mut reap = tokio::time::interval_at(
            tokio::time::Instant::now() + WARM_POOL_REAP_INTERVAL,
            WARM_POOL_REAP_INTERVAL,
        );
        reap.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        self.drive_until_idle(Some(&mut reap)).await?;
        loop {
            tokio::select! {
                () = self.control.shutdown_requested() => return self.drain_shutdown().await,
                () = self.notify.notified() => self.drive_until_idle(Some(&mut reap)).await?,
                _ = reap.tick() => self.reap_warm_pool().await,
            }
        }
    }

    async fn reap_warm_pool(&self) {
        let now = std::time::Instant::now();
        let router = Arc::clone(&self.router);
        let reaped = tokio::task::spawn_blocking(move || {
            router
                .lock()
                .map_err(|_| "ordinary lifecycle router is unavailable".to_owned())?
                .tend_warm_pool(
                    None,
                    &crate::provider_warm_pool::WarmPoolRequest { now, target: None },
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
        if let Ok(mut host) = self.async_claurst.try_lock()
            && let Some(host) = host.as_mut()
            && let Err(error) = host.reap_idle(now).await
        {
            eprintln!("Claurst idle runtime was not stopped: {error}");
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

    async fn drive_until_idle(
        &self,
        mut reap: Option<&mut tokio::time::Interval>,
    ) -> Result<(), String> {
        let shutting_down = reap.is_none();
        let mut wake_idle_hosts =
            !shutting_down && self.claurst_finished.swap(false, Ordering::AcqRel);
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
                    router.wake_idle_hosts();
                }
                let active = router
                    .drive_once()
                    .map_err(|error| format!("ordinary lifecycle drive was rejected: {error:?}"))?;
                Ok::<_, String>((active, router.finished_work()))
            })
            .await
            .map_err(|_| "ordinary lifecycle drive task failed".to_owned())??;
            if router_finished {
                self.async_claurst_notify.notify_one();
            }
            wake_idle_hosts = !shutting_down
                && (router_finished || self.claurst_finished.swap(false, Ordering::AcqRel));
            if !active && !wake_idle_hosts {
                return Ok(());
            }
            match reap.as_deref_mut() {
                Some(reap) => tokio::select! {
                    () = tokio::time::sleep(ACTIVE_POLL_INTERVAL) => {}
                    _ = reap.tick() => self.reap_warm_pool().await,
                },
                None => tokio::time::sleep(ACTIVE_POLL_INTERVAL).await,
            }
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
        self.drive_until_idle(None).await?;
        let router = Arc::clone(&self.router);
        tokio::task::spawn_blocking(move || {
            router
                .lock()
                .map_err(|_| "ordinary lifecycle router is unavailable".to_owned())
                .map(|router| router.shutdown_complete())
        })
        .await
        .map_err(|_| "ordinary lifecycle shutdown task failed".to_owned())??
        .then_some(())
        .ok_or_else(|| "ordinary lifecycle shutdown was not proven by its owner".to_owned())
    }
}
