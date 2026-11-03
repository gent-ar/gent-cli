//! Host-owned routing core. No worker-selected command, environment or target.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use zeroize::Zeroize;

pub const PREFIX: &str = "/__gent_mcp/";
pub const MAX_MESSAGE: usize = 1024 * 1024;
pub type Ticket = Arc<dyn Send + Sync>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Authentication,
    Revoked,
    Route,
    Protocol,
    Denied,
    Approval,
    NotReady,
    Busy,
    Unprovable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Allow,
    Deny,
    Ask,
    NotReady,
}
pub struct Call<'a> {
    pub request_key: &'a str,
    pub scope: Option<&'a gent_ports::HostMcpScope>,
    pub server: &'a str,
    pub tool: Option<&'a str>,
    pub params: &'a Value,
}

/// Implemented by the host policy adapter, borrowing a genuinely published
/// process proof. It must not acquire a runner mutex held during SDK startup.
pub trait Authorizer: Send + Sync {
    fn decide(&self, call: &Call<'_>) -> Decision;
}
/// The production implementation must durably reserve before the effect.
/// Unknown outcomes must never be represented as a fresh operation.
pub enum Admission {
    Fresh,
    Cached(Value),
    Unknown,
}
pub trait Journal: Send + Sync {
    fn capture_scope(&self) -> Result<Option<gent_ports::HostMcpScope>, Error> {
        Ok(None)
    }
    fn current_scope(&self, _: Option<&gent_ports::HostMcpScope>) -> bool {
        true
    }
    fn reserve_bound(
        &self,
        key: &str,
        request: &[u8],
        _: Option<&gent_ports::HostMcpScope>,
    ) -> Result<Admission, Error> {
        self.reserve(key, request)
    }
    fn reserve(&self, key: &str, request: &[u8]) -> Result<Admission, Error>;
    fn complete(&self, key: &str, response: &Value) -> Result<(), Error>;
    fn unknown(&self, key: &str);
}
/// Owns the host stdio process and its descendant scope. No credentials are
/// returned by this interface. Implementations must bound frames and queues.
pub trait Peer: Send + Sync {
    fn notify_bound(
        &self,
        message: &Value,
        current: Arc<dyn Fn() -> bool + Send + Sync>,
        ticket: Option<Ticket>,
    ) -> Result<(), Error> {
        if !current() {
            return Err(Error::Revoked);
        }
        let result = self.notify(message);
        if let Some(ticket) = ticket {
            self.retain_until_settled(ticket);
        }
        result
    }
    /// Default is deliberately fail-closed for implementations without a scope
    /// owner: an uncertain effect must not release maintenance admission.
    fn retain_until_settled(&self, ticket: Ticket) {
        if !self.settled() {
            std::mem::forget(ticket)
        }
    }
    fn request_bound(
        &self,
        message: &Value,
        current: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<Value, Error> {
        if !current() {
            return Err(Error::Revoked);
        }
        self.request(message)
    }
    fn request(&self, message: &Value) -> Result<Value, Error>;
    fn notify(&self, message: &Value) -> Result<(), Error>;
    fn revoke(&self);
    fn settled(&self) -> bool;
}
pub struct Route {
    pub name: String,
    pub peer: Arc<dyn Peer>,
    poisoned: AtomicBool,
    pending: Mutex<BTreeSet<String>>,
}
impl Route {
    pub fn new(name: String, peer: Arc<dyn Peer>) -> Result<Self, Error> {
        if name.is_empty()
            || name.len() > 128
            || name.contains("__")
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
        {
            return Err(Error::Protocol);
        }
        Ok(Self {
            name,
            peer,
            poisoned: AtomicBool::new(false),
            pending: Mutex::new(BTreeSet::new()),
        })
    }
}
pub enum Reply {
    Accepted,
    Json(Value),
    MethodNotAllowed,
}
pub struct Bridge {
    token: String,
    routes: BTreeMap<String, Route>,
    current: Arc<dyn Fn() -> bool + Send + Sync>,
    authorizer: Arc<dyn Authorizer>,
    journal: Arc<dyn Journal>,
    revoked: AtomicBool,
    operation: Option<Arc<dyn Fn() -> Result<Ticket, Error> + Send + Sync>>,
}
impl std::fmt::Debug for Bridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostMcpBridge")
            .field("servers", &self.routes.len())
            .finish_non_exhaustive()
    }
}
impl Bridge {
    pub fn new(
        token: String,
        routes: BTreeMap<String, Route>,
        current: Arc<dyn Fn() -> bool + Send + Sync>,
        authorizer: Arc<dyn Authorizer>,
        journal: Arc<dyn Journal>,
    ) -> Result<Self, Error> {
        if token.len() != 64
            || !token.bytes().all(|c| c.is_ascii_hexdigit())
            || routes.is_empty()
            || routes.len() > 32
            || routes.keys().any(|id| {
                id.len() != 32
                    || !id
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            })
        {
            return Err(Error::Protocol);
        }
        let mut names = std::collections::BTreeSet::new();
        if routes
            .values()
            .any(|r| !names.insert(r.name.replace('-', "_")))
        {
            return Err(Error::Protocol);
        }
        Ok(Self {
            token,
            routes,
            current,
            authorizer,
            journal,
            revoked: AtomicBool::new(false),
            operation: None,
        })
    }
    pub fn with_operations(
        mut self,
        operation: Arc<dyn Fn() -> Result<Ticket, Error> + Send + Sync>,
    ) -> Self {
        self.operation = Some(operation);
        self
    }
    pub fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
        for route in self.routes.values() {
            route.peer.revoke();
        }
    }
    fn guard(
        &self,
        scope: Option<gent_ports::HostMcpScope>,
    ) -> Arc<dyn Fn() -> bool + Send + Sync> {
        let journal = self.journal.clone();
        let current = self.current.clone();
        Arc::new(move || current() && journal.current_scope(scope.as_ref()))
    }
    pub fn settled(&self) -> bool {
        self.revoked.load(Ordering::Acquire) && self.routes.values().all(|r| r.peer.settled())
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.revoke();
        self.token.zeroize();
    }
}
#[path = "provider_mcp_dispatch.rs"]
mod dispatch;
