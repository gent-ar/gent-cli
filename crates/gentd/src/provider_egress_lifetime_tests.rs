use super::*;
use gent_drivers::ProviderProcess;
use gent_drivers::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use std::fs::{File, OpenOptions};
#[path = "provider_egress_lifetime_fixture.rs"]
mod fixture;
use fixture::*;

#[test]
fn unknown_drop_retains_private_root_and_exact_resource_pin() {
    for (stop_ok, exit) in [(false, Ok(Some(0))), (true, Ok(None)), (true, Err(()))] {
        let f = Fixture::new();
        let root = f.lease.private_state.root().to_path_buf();
        let pin = f.ancestor.path().join("pin");
        std::fs::write(&pin, b"pin").unwrap();
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        f.lease
            .retain_resource_handles(vec![options.open(&pin).unwrap()]);
        let weak = Arc::downgrade(&f.lease);
        drop(super::super::bound_process::GatewayBoundProcess {
            process: Arc::new(Process {
                stop_ok,
                exit,
                proof: None,
            }),
            lease: f.lease,
        });
        assert!(root.is_dir());
        let held = weak.upgrade().unwrap();
        assert_eq!(held.retained_resources.lock().unwrap().len(), 1);
        #[cfg(windows)]
        assert!(OpenOptions::new().write(true).open(&pin).is_err());
        super::super::bound_process::release_test_retention(&held);
        drop(held);
        assert!(!root.exists());
        assert!(OpenOptions::new().write(true).open(&pin).is_ok());
    }
}

#[test]
fn proven_drop_releases_root_and_pins_without_quarantine() {
    let f = Fixture::new();
    let root = f.lease.private_state.root().to_path_buf();
    let weak = Arc::downgrade(&f.lease);
    f.lease
        .retain_resource_handles(vec![File::open(&f.launch.executable).unwrap()]);
    drop(super::super::bound_process::GatewayBoundProcess {
        process: Arc::new(Process {
            stop_ok: true,
            exit: Ok(Some(0)),
            proof: None,
        }),
        lease: f.lease,
    });
    assert!(weak.upgrade().is_none());
    assert!(!root.exists());
}

struct Recorded(std::sync::Mutex<Vec<ProcessTreeSignal>>);
impl ProcessTreeControl for Recorded {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        self.0.lock().unwrap().push(signal);
        Ok(())
    }
}
impl ProviderProcess for Recorded {
    fn write_frame(&self, _: &[u8]) -> Result<(), ProcessTreeError> {
        Ok(())
    }
    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        Ok(())
    }
    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        Ok(Some(Some(0)))
    }
}

#[test]
fn a_stale_binding_and_drop_request_the_settling_stop_never_the_unprovable_kill() {
    let f = Fixture::new();
    let process = Arc::new(Recorded(std::sync::Mutex::new(Vec::new())));
    let bound = super::super::bound_process::GatewayBoundProcess {
        process: process.clone(),
        lease: f.lease,
    };
    bound.lease.revoke();
    assert!(bound.write_frame(b"{}\n").is_err());
    drop(bound);
    let signals = process.0.lock().unwrap();
    assert!(!signals.is_empty());
    assert!(!signals.contains(&ProcessTreeSignal::Kill));
}

#[test]
fn failed_launch_retains_lease_before_a_wrapper_exists() {
    let f = Fixture::new();
    let root = f.lease.private_state.root().to_path_buf();
    let weak = Arc::downgrade(&f.lease);
    let launcher = f
        .lease
        .launcher(
            Backend {
                revoke: weak.clone(),
                fail: true,
            },
            limits(),
            vec![],
        )
        .unwrap();
    assert!(launcher.launch(&f.launch).is_err());
    drop(launcher);
    drop(f.lease);
    let held = weak.upgrade().unwrap();
    assert!(root.is_dir());
    super::super::bound_process::release_test_retention(&held);
    drop(held);
    assert!(!root.exists());
}

#[test]
fn a_selection_switch_keeps_the_superseded_runs_egress_fence_current() {
    let f = Fixture::new();
    let fence = &f.lease.fence;
    gent_ports::AgentChatSelectionLedger::switch_agent_chat_selection(
        &fence.ledger,
        &gent_types::AgentChatSelectionSwitch {
            receipt_id: gent_types::ReceiptId("switch".into()),
            idempotency_key: "switch".into(),
            host_epoch: gent_types::HostEpoch(1),
            conversation_id: gent_types::AgentChatConversationId("conversation".into()),
            parent_run_id: gent_types::AgentChatRunId("run".into()),
            run_id: gent_types::AgentChatRunId("next".into()),
            selection: gent_types::AgentChatSelection {
                provider: gent_types::AgentChatProvider::Codex,
                model: "gpt-5.6-luna".into(),
                effort: gent_types::AgentChatEffort::Low,
                mode: gent_types::AgentChatMode::Ask,
            },
            context_policy: gent_types::ContextPolicy::Preserve,
        },
    )
    .unwrap();
    assert!(fence.current());
    assert!(fence.matches(&f.launch));
    let claimed = f.launch.permission_binding.as_ref().unwrap();
    assert!(
        RunFence::capture(
            fence.ledger.clone(),
            fence.coordinator.clone(),
            claimed,
            f.launch.lock.clone(),
        )
        .is_ok()
    );
}

#[test]
fn a_policy_saved_after_claim_keeps_the_sandboxed_launch_and_reaches_the_next_prompt() {
    let f = Fixture::new();
    let fence = &f.lease.fence;
    let claimed = f.launch.permission_binding.clone().unwrap();
    let mut changed = claimed.policy().clone();
    changed.revision += 1;
    changed.policy_id = "policy-revision-2".into();
    changed.mode = gent_types::PermissionMode::AskEveryTime;
    gent_ports::PolicyLedger::save_policy(&fence.ledger, &changed).unwrap();
    let capture = |binding: &gent_types::SandboxPermissionBinding| {
        RunFence::capture(
            fence.ledger.clone(),
            fence.coordinator.clone(),
            binding,
            f.launch.lock.clone(),
        )
        .unwrap()
    };
    let launching = capture(&claimed);
    assert_eq!(launching.policy, *claimed.policy());
    assert!(launching.matches(&f.launch));
    let next_policy = crate::permission_workspace::policy_for_conversation(
        &fence.ledger,
        "conversation",
        "workspace",
    )
    .unwrap();
    assert_eq!(
        (next_policy.policy_id.as_str(), next_policy.mode),
        (
            "policy-revision-2",
            gent_types::PermissionMode::AskEveryTime
        )
    );
    let next = gent_types::SandboxPermissionBinding::new(
        "conversation".into(),
        "run".into(),
        next_policy.clone(),
        gent_types::HostEpoch(1),
    )
    .unwrap();
    assert_eq!(capture(&next).policy, next_policy);
}

#[test]
fn postspawn_revocation_wraps_before_fallible_authority_recheck() {
    let f = Fixture::new();
    let root = f.lease.private_state.root().to_path_buf();
    let weak = Arc::downgrade(&f.lease);
    let launcher = f
        .lease
        .launcher(
            Backend {
                revoke: weak.clone(),
                fail: false,
            },
            limits(),
            vec![],
        )
        .unwrap();
    assert!(launcher.launch(&f.launch).is_err());
    drop(launcher);
    drop(f.lease);
    let held = weak.upgrade().unwrap();
    assert!(root.is_dir());
    super::super::bound_process::release_test_retention(&held);
    drop(held);
    assert!(!root.exists());
}
