use super::*;

fn plan(writable: &[&str]) -> LinuxBubblewrapMountPlan {
    LinuxBubblewrapMountPlan::compile(
        "/work/project".into(),
        vec![
            "/work/project".into(),
            "/opt/runtime/node".into(),
            "/private/run".into(),
        ],
        writable.iter().map(|path| (*path).into()).collect(),
    )
    .unwrap()
}

fn mounts(plan: &LinuxBubblewrapMountPlan, option: &str) -> Vec<(String, String)> {
    plan.arguments()
        .windows(3)
        .filter(|window| window[0] == option)
        .map(|window| (window[1].clone(), window[2].clone()))
        .collect()
}

#[test]
fn readonly_workspace_keeps_only_explicit_private_state_writable() {
    let actual = plan(&["/private/run"]);
    assert_eq!(
        mounts(&actual, "--bind"),
        vec![("/private/run".into(), "/private/run".into())]
    );
    assert!(
        mounts(&actual, "--ro-bind").contains(&("/work/project".into(), "/work/project".into()))
    );
}

#[test]
fn agent_workspace_write_never_grants_its_parent_or_runtime() {
    let actual = plan(&["/work/project", "/private/run"]);
    assert_eq!(mounts(&actual, "--bind").len(), 2);
    assert!(
        !mounts(&actual, "--bind")
            .iter()
            .any(|(path, _)| path == "/work" || path == "/opt/runtime/node")
    );
    let last_read = actual
        .arguments()
        .iter()
        .rposition(|arg| arg == "--ro-bind")
        .unwrap();
    let first_write = actual
        .arguments()
        .iter()
        .position(|arg| arg == "--bind")
        .unwrap();
    assert!(last_read < first_write);
}

#[test]
fn namespace_flags_are_required_and_never_best_effort() {
    let actual = plan(&[]);
    for flag in [
        "--unshare-user",
        "--unshare-pid",
        "--unshare-net",
        "--unshare-ipc",
        "--unshare-uts",
        "--unshare-cgroup",
        "--disable-userns",
        "--clearenv",
        "--new-session",
        "--die-with-parent",
    ] {
        assert!(actual.arguments().iter().any(|arg| arg == flag));
    }
    assert!(
        !actual
            .arguments()
            .iter()
            .any(|arg| arg.ends_with("-try") || arg == "--share-net")
    );
    assert!(
        !mounts(&actual, "--ro-bind")
            .iter()
            .any(|(path, _)| path == "/")
    );
}

#[test]
fn paths_cannot_inject_options_or_escape_namespace_filesystems() {
    for path in [
        "",
        "/",
        "relative",
        "--share-net",
        "C:/work",
        "/work/../etc",
        "/work/./file",
        "//server/share",
        "/work//file",
        "/work/",
        "/work\0file",
    ] {
        assert!(
            LinuxBubblewrapMountPlan::compile(path.into(), vec![path.into()], vec![]).is_err(),
            "{path:?}"
        );
    }
    for path in ["/proc", "/proc/self/fd", "/dev", "/dev/shm", "/sys/kernel"] {
        assert_eq!(
            LinuxBubblewrapMountPlan::compile(path.into(), vec![path.into()], vec![]),
            Err(LinuxMountPlanError::ReservedPath)
        );
    }
}

#[test]
fn shell_metacharacters_remain_one_exact_argument() {
    let path = "/work/a ; $(touch sentinel) 'quoted'";
    let actual = LinuxBubblewrapMountPlan::compile(path.into(), vec![path.into()], vec![]).unwrap();
    assert_eq!(
        mounts(&actual, "--ro-bind"),
        vec![(path.into(), path.into())]
    );
    assert_eq!(actual.arguments().last().unwrap(), path);
}

#[test]
fn writable_prefix_collision_is_rejected_and_order_is_deterministic() {
    assert_eq!(
        LinuxBubblewrapMountPlan::compile(
            "/work".into(),
            vec!["/work".into()],
            vec!["/workspace".into()]
        ),
        Err(LinuxMountPlanError::InvalidWritableMount)
    );
    let first = LinuxBubblewrapMountPlan::compile(
        "/work".into(),
        vec!["/work/sub".into(), "/work".into(), "/work".into()],
        vec![],
    )
    .unwrap();
    let second = LinuxBubblewrapMountPlan::compile(
        "/work".into(),
        vec!["/work".into(), "/work/sub".into()],
        vec![],
    )
    .unwrap();
    assert_eq!(first, second);
}
#[test]
fn profile_adapter_refuses_reviewed_egress_instead_of_sharing_host_network() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    let profile = gent_types::SandboxLaunchProfile::new(
        &path,
        std::slice::from_ref(&path),
        &[],
        vec![],
        gent_types::SandboxNetworkPolicy::ReviewedEgress {
            policy_digest_sha256: "a".repeat(64),
        },
        gent_types::SandboxResourceLimits {
            max_processes: 4,
            max_memory_bytes: 1024 * 1024,
            max_cpu_time_ms: 1000,
        },
    )
    .unwrap();
    assert_eq!(
        LinuxBubblewrapMountPlan::from_profile(&profile),
        Err(LinuxMountPlanError::UnsupportedNetwork)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn profile_adapter_preserves_actual_linux_workspace_access() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    for writable in [false, true] {
        let writes = if writable { vec![path.clone()] } else { vec![] };
        let profile = gent_types::SandboxLaunchProfile::new(
            &path,
            std::slice::from_ref(&path),
            &writes,
            vec![],
            gent_types::SandboxNetworkPolicy::Disabled,
            gent_types::SandboxResourceLimits {
                max_processes: 4,
                max_memory_bytes: 1024 * 1024,
                max_cpu_time_ms: 1000,
            },
        )
        .unwrap();
        let actual = LinuxBubblewrapMountPlan::from_profile(&profile).unwrap();
        assert_eq!(mounts(&actual, "--bind").len(), usize::from(writable));
        assert!(
            mounts(&actual, "--ro-bind")
                .contains(&(path.to_str().unwrap().into(), path.to_str().unwrap().into()))
        );
    }
}
#[test]
fn owned_kernel_fixture_exports_the_same_mount_plan() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = format!("/tmp/gent-linux-mount-fixture-{nonce:032x}");
    let work = format!("{root}/work");
    let private = format!("{root}/private");
    let runtime = format!("{root}/runtime");
    let reads = vec![
        work.clone(),
        private.clone(),
        runtime,
        "/usr".into(),
        "/bin".into(),
        "/lib".into(),
        "/lib64".into(),
    ];
    let readonly =
        LinuxBubblewrapMountPlan::compile(work.clone(), reads.clone(), vec![private.clone()])
            .unwrap();
    let readwrite =
        LinuxBubblewrapMountPlan::compile(work.clone(), reads, vec![private, work]).unwrap();
    assert_eq!(mounts(&readonly, "--bind").len(), 1);
    assert_eq!(mounts(&readwrite, "--bind").len(), 2);
    println!(
        "GENT_LINUX_MOUNT_PLAN={}",
        serde_json::json!({"root": root, "readOnly": readonly.arguments(), "readWrite": readwrite.arguments()})
    );
}
