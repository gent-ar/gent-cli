use super::{mcp::*, mcp_operations::acquire};
use crate::ordinary_lifecycle_control::OrdinaryLifecycleControl;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
struct Allow;
impl Authorizer for Allow {
    fn decide(&self, _: &Call<'_>) -> Decision {
        Decision::Allow
    }
}
struct Log;
impl Journal for Log {
    fn reserve(&self, _: &str, _: &[u8]) -> Result<Admission, Error> {
        Ok(Admission::Fresh)
    }
    fn complete(&self, _: &str, _: &Value) -> Result<(), Error> {
        Err(Error::Unprovable)
    }
    fn unknown(&self, _: &str) {}
}
struct Scope {
    terminal: AtomicBool,
    effects: AtomicUsize,
    settled: AtomicBool,
    held: Mutex<Vec<Ticket>>,
}
impl Peer for Scope {
    fn request(&self, v: &Value) -> Result<Value, Error> {
        self.effects.fetch_add(1, Ordering::SeqCst);
        self.terminal.store(true, Ordering::SeqCst);
        Ok(json!({"jsonrpc":"2.0","id":v["id"],"result":{}}))
    }
    fn notify(&self, _: &Value) -> Result<(), Error> {
        Ok(())
    }
    fn revoke(&self) {}
    fn settled(&self) -> bool {
        self.settled.load(Ordering::SeqCst)
    }
    fn retain_until_settled(&self, ticket: Ticket) {
        self.held.lock().unwrap().push(ticket)
    }
}
fn router(peer: Arc<Scope>, control: OrdinaryLifecycleControl) -> Bridge {
    Bridge::new(
        "a".repeat(64),
        BTreeMap::from([("b".repeat(32), Route::new("server".into(), peer).unwrap())]),
        Arc::new(|| true),
        Arc::new(Allow),
        Arc::new(Log),
    )
    .unwrap()
    .with_operations(Arc::new(move || acquire(&control)))
}
fn invoke(b: &Bridge) -> Result<Reply, Error> {
    b.dispatch(
        &format!("Bearer {}", "a".repeat(64)),
        None,
        &format!("{PREFIX}{}", "b".repeat(32)),
        "POST",
        br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"write"}}"#,
    )
}
fn peer() -> Arc<Scope> {
    Arc::new(Scope {
        terminal: AtomicBool::new(false),
        effects: AtomicUsize::new(0),
        settled: AtomicBool::new(false),
        held: Mutex::new(Vec::new()),
    })
}
#[test]
fn host_mcp_operation_unknown_blocks_quiesce_after_terminal_until_owned_scope_settles() {
    let control = OrdinaryLifecycleControl::new();
    control.open_after_recovery();
    let peer = peer();
    let b = router(peer.clone(), control.clone());
    assert!(matches!(invoke(&b), Err(Error::Unprovable)));
    assert!(peer.terminal.load(Ordering::SeqCst));
    assert_eq!(peer.effects.load(Ordering::SeqCst), 1);
    // The turn is terminal; the outstanding owner permit alone prevents cutover.
    assert!(control.quiesce_if_idle(|| Ok(())).is_err());
    peer.settled.store(true, Ordering::SeqCst);
    peer.held.lock().unwrap().clear();
    assert!(control.quiesce_if_idle(|| Ok(())).is_ok());
}
#[test]
fn host_mcp_operation_closed_admission_prevents_any_effect() {
    let control = OrdinaryLifecycleControl::new();
    control.open_after_recovery();
    control.quiesce_if_idle(|| Ok(())).unwrap();
    let peer = peer();
    let b = router(peer.clone(), control);
    assert!(matches!(invoke(&b), Err(Error::Revoked)));
    assert_eq!(peer.effects.load(Ordering::SeqCst), 0);
}
