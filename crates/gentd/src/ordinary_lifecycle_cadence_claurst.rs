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
        self.async_claurst
            .lock()
            .await
            .as_mut()
            .ok_or_else(|| "Claurst permission owner is unavailable".to_owned())?
            .respond_claurst_permission(response)
            .await
    }

    pub(crate) async fn respond_claurst_permission_with_receipt(
        &self,
        response: gent_types::PermissionDecisionResponse,
        receipt_id: gent_types::ReceiptId,
    ) -> Result<gent_types::Receipt, String> {
        self.async_claurst
            .lock()
            .await
            .as_mut()
            .ok_or_else(|| "Claurst permission owner is unavailable".to_owned())?
            .respond_claurst_permission_with_receipt(response, receipt_id)
            .await
    }
}
