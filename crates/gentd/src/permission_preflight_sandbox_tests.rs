use super::{PermissionPreflight, evaluate_with_evidence};
use gent_drivers::ProcessSandboxEvidence;
use gent_types::{
    HostEpoch, PermissionCategory, PermissionMode, PermissionRequest, PolicyRecord, PolicyScope,
    RunVersionLock, SandboxBackendId, SandboxLaunchProfile, SandboxNetworkPolicy,
    SandboxPermissionBinding, SandboxResourceLimits, SandboxedLaunchRequest,
};

#[test]
fn broad_permission_requires_the_exact_run_and_a_broad_launch_of_the_same_workspace() {
    let root = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(root.path()).unwrap();
    let policy = bypass_policy();
    let lock = test_lock();
    let proof = evidence(&root, &lock, &policy, "123:456");
    let request = PermissionRequest::new("Bash".into(), PermissionCategory::Command, None, None);
    assert_eq!(
        evaluate_with_evidence(&policy, &request, "run", Some(&proof)),
        PermissionPreflight::Allow
    );
    assert_eq!(
        evaluate_with_evidence(&policy, &request, "other-run", Some(&proof)),
        PermissionPreflight::Ask
    );
    assert_eq!(
        evaluate_with_evidence(&policy, &request, "run", None),
        PermissionPreflight::Ask
    );
    let control = PermissionRequest::new(
        "mcp__gent_goal__gent_goal_update".into(),
        PermissionCategory::Provider,
        None,
        None,
    );
    for mode in [PermissionMode::Autonomous, PermissionMode::Bypass] {
        let mut current = policy.clone();
        current.mode = mode;
        let valid = evidence(&root, &lock, &current, "fixture-generation");
        let question = PermissionRequest::new(
            "AskUserQuestion".into(),
            PermissionCategory::Provider,
            None,
            None,
        );
        assert_eq!(
            super::evaluate_callback(
                &current,
                &question,
                "run",
                "conversation",
                HostEpoch(1),
                Some(&valid),
                true
            ),
            PermissionPreflight::Ask
        );
        assert_eq!(
            super::evaluate_callback(
                &current,
                &control,
                "run",
                "conversation",
                HostEpoch(1),
                Some(&valid),
                true
            ),
            PermissionPreflight::Allow
        );
        assert_eq!(
            super::evaluate_callback(
                &current,
                &control,
                "run",
                "conversation",
                HostEpoch(1),
                None,
                true
            ),
            PermissionPreflight::Ask
        );
        assert_eq!(
            super::evaluate_callback(
                &current,
                &control,
                "run",
                "conversation",
                HostEpoch(2),
                Some(&valid),
                true
            ),
            PermissionPreflight::Ask
        );
        assert_eq!(
            super::evaluate_callback(
                &current,
                &control,
                "run",
                "other-conversation",
                HostEpoch(1),
                Some(&valid),
                true
            ),
            PermissionPreflight::Ask
        );
        let mut changed = current.clone();
        changed.revision += 1;
        assert_eq!(
            super::evaluate_callback(
                &changed,
                &control,
                "run",
                "conversation",
                HostEpoch(1),
                Some(&valid),
                true
            ),
            PermissionPreflight::Allow
        );
    }
    let mut ask = policy.clone();
    ask.mode = PermissionMode::AskEveryTime;
    assert_eq!(
        super::evaluate_callback(
            &ask,
            &control,
            "run",
            "conversation",
            HostEpoch(1),
            None,
            true
        ),
        PermissionPreflight::Allow
    );
}

#[test]
fn evidence_covers_a_revised_policy_of_the_launch_workspace_only_after_a_broad_launch() {
    let root = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(root.path()).unwrap();
    let policy = bypass_policy();
    let lock = test_lock();
    let proof = evidence(&root, &lock, &policy, "123:456");
    let request = PermissionRequest::new("Bash".into(), PermissionCategory::Command, None, None);
    for change in 0..4 {
        let mut changed = policy.clone();
        let covered = match change {
            0 => {
                changed.revision += 1;
                true
            }
            1 => {
                changed.policy_id.push_str("-changed");
                true
            }
            2 => {
                changed.allowed_tools.push("Read".into());
                true
            }
            _ => {
                changed.workspace_id.push_str("-changed");
                false
            }
        };
        assert_eq!(
            evaluate_with_evidence(&changed, &request, "run", Some(&proof)),
            if covered {
                PermissionPreflight::Allow
            } else {
                PermissionPreflight::Ask
            }
        );
    }
    let mut ask = policy.clone();
    ask.mode = PermissionMode::AskEveryTime;
    let narrow = evidence(&root, &lock, &ask, "ask-launch");
    assert_eq!(
        evaluate_with_evidence(&policy, &request, "run", Some(&narrow)),
        PermissionPreflight::Ask
    );
}

fn evidence(
    root: &std::path::Path,
    lock: &RunVersionLock,
    policy: &PolicyRecord,
    generation: &str,
) -> ProcessSandboxEvidence {
    let binding = SandboxPermissionBinding::new(
        "conversation".into(),
        "run".into(),
        policy.clone(),
        HostEpoch(1),
    )
    .unwrap();
    let profile = SandboxLaunchProfile::new(
        root,
        &[root.to_path_buf()],
        &[root.to_path_buf()],
        vec![],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 1,
            max_memory_bytes: 1024,
            max_cpu_time_ms: 1000,
        },
    )
    .unwrap()
    .with_permission_binding(binding);
    let launch = SandboxedLaunchRequest {
        lock: lock.clone(),
        profile,
    };
    ProcessSandboxEvidence::new(
        &launch,
        lock,
        "run".into(),
        generation.into(),
        SandboxBackendId::new("test-backend".into()).unwrap(),
    )
    .unwrap()
}

fn test_lock() -> RunVersionLock {
    RunVersionLock {
        provider: "claude".into(),
        canonical_path: "test-executable".into(),
        digest_sha256: "a".repeat(64),
        file_identity: "test-file".into(),
        version: "1".into(),
        compatibility_entry: "test".into(),
    }
}

fn bypass_policy() -> PolicyRecord {
    PolicyRecord {
        policy_id: "policy".into(),
        workspace_id: "workspace".into(),
        scope: PolicyScope::ProviderPermissions,
        revision: 1,
        mode: PermissionMode::Bypass,
        containment: gent_types::Containment::Sandboxed,
        allowed_tools: vec![],
        allowed_categories: vec![],
        allowed_commands: Vec::new(),
    }
}
