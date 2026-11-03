//! Deterministic T1 authorization / T2 replacement interleaving.
use super::mcp::*;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
fn scope(turn: &str) -> gent_ports::HostMcpScope {
    gent_ports::HostMcpScope {
        conversation_id: "c".into(),
        run_id: "r".into(),
        turn_id: turn.into(),
        coordinator_id: "owner".into(),
        host_epoch: gent_types::HostEpoch(1),
        policy_id: "p".into(),
        source_sha256: "a".repeat(64),
        generation: "b".repeat(32),
    }
}
struct JournalProbe {
    current: Arc<Mutex<gent_ports::HostMcpScope>>,
    reserved: Arc<AtomicUsize>,
}
impl Journal for JournalProbe {
    fn capture_scope(&self) -> Result<Option<gent_ports::HostMcpScope>, Error> {
        Ok(Some(self.current.lock().unwrap().clone()))
    }
    fn current_scope(&self, s: Option<&gent_ports::HostMcpScope>) -> bool {
        s == Some(&*self.current.lock().unwrap())
    }
    fn reserve(&self, _: &str, _: &[u8]) -> Result<Admission, Error> {
        panic!("unbound reservation forbidden")
    }
    fn reserve_bound(
        &self,
        _: &str,
        _: &[u8],
        s: Option<&gent_ports::HostMcpScope>,
    ) -> Result<Admission, Error> {
        if !self.current_scope(s) {
            return Err(Error::Revoked);
        }
        self.reserved.fetch_add(1, Ordering::SeqCst);
        Ok(Admission::Fresh)
    }
    fn complete(&self, _: &str, _: &Value) -> Result<(), Error> {
        Ok(())
    }
    fn unknown(&self, _: &str) {}
}
struct SwapAfterAuthorization(Arc<Mutex<gent_ports::HostMcpScope>>);
impl Authorizer for SwapAfterAuthorization {
    fn decide(&self, call: &Call<'_>) -> Decision {
        assert_eq!(call.scope.unwrap().turn_id, "T1");
        *self.0.lock().unwrap() = scope("T2");
        Decision::Allow
    }
}
struct Counter(Arc<AtomicUsize>);
impl Peer for Counter {
    fn request(&self, _: &Value) -> Result<Value, Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        unreachable!("stale T1 must not write")
    }
    fn notify(&self, _: &Value) -> Result<(), Error> {
        Ok(())
    }
    fn revoke(&self) {}
    fn settled(&self) -> bool {
        true
    }
}
#[test]
fn host_mcp_scope_turn_swap_after_grant_never_reserves_or_writes_as_successor() {
    let current = Arc::new(Mutex::new(scope("T1")));
    let reserved = Arc::new(AtomicUsize::new(0));
    let effects = Arc::new(AtomicUsize::new(0));
    let route = "a".repeat(32);
    let bridge = Bridge::new(
        "b".repeat(64),
        BTreeMap::from([(
            route.clone(),
            Route::new("server".into(), Arc::new(Counter(effects.clone()))).unwrap(),
        )]),
        Arc::new(|| true),
        Arc::new(SwapAfterAuthorization(current.clone())),
        Arc::new(JournalProbe {
            current,
            reserved: reserved.clone(),
        }),
    )
    .unwrap();
    let body=serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"write","arguments":{}}})).unwrap();
    assert!(matches!(
        bridge.dispatch(
            &format!("Bearer {}", "b".repeat(64)),
            None,
            &format!("{PREFIX}{route}"),
            "POST",
            &body
        ),
        Err(Error::Revoked)
    ));
    assert_eq!(reserved.load(Ordering::SeqCst), 0);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
}
