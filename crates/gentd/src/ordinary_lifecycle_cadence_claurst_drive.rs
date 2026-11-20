use std::sync::{Arc, atomic::Ordering};

use gent_drivers::provider_activity::{ActivityWaker, subscribe};

use super::OrdinaryLifecycleCadence;

impl<L: Send + 'static> OrdinaryLifecycleCadence<L> {
    pub(super) async fn run_claurst(&self) -> Result<(), String> {
        let notify = Arc::clone(&self.async_claurst_notify);
        let waker: Arc<ActivityWaker> = Arc::new(move || notify.notify_one());
        subscribe(&waker);
        let mut working = false;
        loop {
            self.drive_claurst(false, &mut working).await?;
            tokio::select! {
                () = self.control.shutdown_requested() => break,
                () = self.async_claurst_notify.notified() => {}
            }
        }
        self.control.wait_for_permits().await;
        if let Some(host) = self.async_claurst.lock().await.as_mut() {
            host.begin_shutdown_after_recovery().await?;
        }
        loop {
            self.drive_claurst(true, &mut working).await?;
            let stopped = self
                .async_claurst
                .lock()
                .await
                .as_ref()
                .is_none_or(|host| host.shutdown_complete());
            if stopped {
                return Ok(());
            }
            self.async_claurst_notify.notified().await;
        }
    }

    async fn drive_claurst(&self, shutting_down: bool, working: &mut bool) -> Result<(), String> {
        loop {
            if !shutting_down && self.control.is_quiesced() {
                return Ok(());
            }
            let mut guard = self.async_claurst.lock().await;
            let Some(host) = guard.as_mut() else {
                return Ok(());
            };
            let progressed = host.drive_once().await?;
            let idle = host.is_idle();
            if std::mem::replace(working, !idle) && idle && !shutting_down {
                self.claurst_finished.store(true, Ordering::Release);
                self.notify.notify_one();
            }
            let warm = self
                .async_claurst_warm
                .lock()
                .map_err(|_| "Claurst warm-up slot is unavailable".to_owned())?
                .take();
            if let Some(target) = warm.filter(|_| idle) {
                host.warm_claurst(&target.conversation_id, &target.run_id);
            }
            if !progressed {
                return Ok(());
            }
        }
    }
}
