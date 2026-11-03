//! Bridges the routing core to the host's durable ledger, never a memory fallback.
use super::mcp::{Admission, Error, Journal};
use gent_ports::{HostMcpAdmission, HostMcpJournal, HostMcpScope};
use serde_json::Value;
use std::sync::Arc;

pub struct DurableJournal {
    ledger: Arc<dyn HostMcpJournal>,
    // The live owner selects the current active turn; never accept worker IDs.
    scope: Arc<dyn Fn() -> Option<HostMcpScope> + Send + Sync>,
    reserved: std::sync::Mutex<std::collections::BTreeMap<String, HostMcpScope>>,
}
impl DurableJournal {
    pub fn new(
        ledger: Arc<dyn HostMcpJournal>,
        scope: Arc<dyn Fn() -> Option<HostMcpScope> + Send + Sync>,
    ) -> Self {
        Self {
            ledger,
            scope,
            reserved: std::sync::Mutex::default(),
        }
    }
}
impl Journal for DurableJournal {
    fn capture_scope(&self) -> Result<Option<HostMcpScope>, Error> {
        Ok(Some((self.scope)().ok_or(Error::Revoked)?))
    }
    fn current_scope(&self, scope: Option<&HostMcpScope>) -> bool {
        scope.is_some_and(|expected| (self.scope)().as_ref() == Some(expected))
    }
    fn reserve_bound(
        &self,
        key: &str,
        request: &[u8],
        scope: Option<&HostMcpScope>,
    ) -> Result<Admission, Error> {
        let scope = scope.ok_or(Error::Revoked)?.clone();
        if !self.current_scope(Some(&scope)) {
            return Err(Error::Revoked);
        }
        self.reserve_captured(key, request, scope)
    }
    fn reserve(&self, key: &str, request: &[u8]) -> Result<Admission, Error> {
        let scope = (self.scope)().ok_or(Error::Revoked)?;
        self.reserve_captured(key, request, scope)
    }
    fn complete(&self, key: &str, response: &Value) -> Result<(), Error> {
        let mut pending = self.reserved.lock().map_err(|_| Error::Unprovable)?;
        let scope = pending.get(key).ok_or(Error::Unprovable)?;
        let bytes = serde_json::to_vec(response).map_err(|_| Error::Protocol)?;
        self.ledger
            .complete_host_mcp(scope, key, &bytes)
            .map_err(|_| Error::Unprovable)?;
        pending.remove(key);
        Ok(())
    }
    fn unknown(&self, key: &str) {
        if let Ok(mut pending) = self.reserved.lock() {
            if let Some(scope) = pending.remove(key) {
                let _ = self.ledger.unknown_host_mcp(&scope, key);
            }
        }
    }
}
impl DurableJournal {
    fn reserve_captured(
        &self,
        key: &str,
        request: &[u8],
        scope: HostMcpScope,
    ) -> Result<Admission, Error> {
        let mut pending = self.reserved.lock().map_err(|_| Error::Unprovable)?;
        if pending.contains_key(key) {
            return Err(Error::Busy);
        }
        match self
            .ledger
            .reserve_host_mcp(&scope, key, request)
            .map_err(|_| Error::Unprovable)?
        {
            HostMcpAdmission::Fresh => {
                pending.insert(key.into(), scope);
                Ok(Admission::Fresh)
            }
            HostMcpAdmission::Completed(bytes) => Ok(Admission::Cached(
                serde_json::from_slice(&bytes).map_err(|_| Error::Unprovable)?,
            )),
            HostMcpAdmission::Unknown => Ok(Admission::Unknown),
        }
    }
}
impl Drop for DurableJournal {
    fn drop(&mut self) {
        if let Ok(pending) = self.reserved.get_mut() {
            for (key, scope) in pending.iter() {
                let _ = self.ledger.unknown_host_mcp(scope, key);
            }
        }
    }
}
