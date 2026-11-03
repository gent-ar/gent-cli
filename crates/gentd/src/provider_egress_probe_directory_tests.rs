use super::*;
#[test]
#[cfg_attr(windows, ignore = "requires exact pinned native cleanup cohort")]
fn successful_settled_probe_cleans_only_its_owned_tree() {
    let parent = tempfile::tempdir().unwrap();
    let outside = parent.path().join("outside");
    std::fs::write(&outside, b"unchanged").unwrap();
    let mut directory = Directory::create(parent.path()).unwrap();
    let root = directory.path().to_path_buf();
    std::fs::create_dir(root.join("cwd")).unwrap();
    directory.in_flight();
    directory.settled();
    directory.cleanup().unwrap();
    assert!(!root.exists());
    assert_eq!(std::fs::read(outside).unwrap(), b"unchanged");
}
#[test]
#[cfg_attr(windows, ignore = "requires exact pinned native cleanup cohort")]
fn known_failure_cleans_but_unknown_failure_retains() {
    let parent = tempfile::tempdir().unwrap();
    let known = {
        let mut directory = Directory::create(parent.path()).unwrap();
        let root = directory.path().to_path_buf();
        directory.in_flight();
        directory.settled();
        root
    };
    assert!(!known.exists());
    let mut unknown = Directory::create(parent.path()).unwrap();
    let root = unknown.path().to_path_buf();
    unknown.in_flight();
    assert!(unknown.cleanup().is_err());
    assert!(root.exists());
    unknown.settled();
    unknown.cleanup().unwrap();
}
#[cfg(windows)]
#[test]
#[cfg_attr(windows, ignore = "requires exact pinned native cleanup cohort")]
fn junction_is_retained_and_never_followed() {
    let parent = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("canary"), b"present").unwrap();
    let mut directory = Directory::create(parent.path()).unwrap();
    let root = directory.path().to_path_buf();
    let link = root.join("escape");
    // cmd's own raw-command parser misparses a slash-spelled argv[0].
    // Use the ordinary native Windows spelling; this is fixture setup.
    let cmd = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32")
        .join("cmd.exe");
    let status = std::process::Command::new(cmd)
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&link)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "junction fixture: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(directory.cleanup().is_err());
    assert_eq!(
        std::fs::read(outside.path().join("canary")).unwrap(),
        b"present"
    );
    std::fs::remove_dir(link).unwrap();
    directory.cleanup().unwrap();
}
#[test]
#[cfg_attr(windows, ignore = "requires exact pinned native cleanup cohort")]
fn sdk_regular_artifact_requires_known_scope_then_is_cleaned() {
    let parent = tempfile::tempdir().unwrap();
    let mut directory = Directory::create(parent.path()).unwrap();
    let root = directory.path().to_path_buf();
    std::fs::create_dir(root.join("fswatch-probe-public")).unwrap();
    let file = root.join("fswatch-probe-public/probe");
    std::fs::write(&file, b"x").unwrap();
    directory.in_flight();
    assert!(directory.cleanup().is_err());
    assert_eq!(std::fs::metadata(&file).unwrap().len(), 1);
    directory.settled();
    directory.cleanup().unwrap();
    assert!(!root.exists());
}

#[test]
#[cfg_attr(windows, ignore = "requires exact pinned native cleanup cohort")]
fn oversized_artifact_rejects_complete_tree_before_mutation() {
    let parent = tempfile::tempdir().unwrap();
    let mut directory = Directory::create(parent.path()).unwrap();
    let ordinary = directory.path().join("ordinary");
    std::fs::write(&ordinary, b"retain").unwrap();
    let oversized = directory.path().join("oversized");
    File::create(&oversized)
        .unwrap()
        .set_len(MAX_FILE_BYTES + 1)
        .unwrap();
    assert!(directory.cleanup().is_err());
    assert!(ordinary.exists() && oversized.exists());
    std::fs::remove_file(oversized).unwrap();
    directory.cleanup().unwrap();
}

#[test]
#[cfg_attr(windows, ignore = "requires exact pinned native cleanup cohort")]
fn hardlink_is_retained_and_outside_object_is_unchanged() {
    let parent = tempfile::tempdir().unwrap();
    let outside = parent.path().join("outside");
    std::fs::write(&outside, b"canary").unwrap();
    let mut directory = Directory::create(parent.path()).unwrap();
    let link = directory.path().join("hardlink");
    // Construct the hostile artifact before retaining the root capability:
    // Windows deliberately refuses new hardlink creation while that cap lives.
    drop(directory.lock.take());
    std::fs::hard_link(&outside, &link).unwrap();
    let root_lock = open_child(&directory.parent, directory.path()).unwrap();
    assert!(
        directory
            .birth
            .same(&Snapshot::capture(&root_lock).unwrap())
    );
    directory.lock = Some(root_lock);
    assert!(directory.cleanup().is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"canary");
    std::fs::remove_file(link).unwrap();
    directory.cleanup().unwrap();
}

#[test]
#[cfg_attr(windows, ignore = "requires exact pinned native cleanup cohort")]
fn expired_deadline_rejects_scan_without_mutation() {
    let parent = tempfile::tempdir().unwrap();
    let mut directory = Directory::create(parent.path()).unwrap();
    let file = directory.path().join("artifact");
    std::fs::write(&file, b"x").unwrap();
    let deadline = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .unwrap();
    let mut walk = Walk {
        volume: directory.birth.volume,
        deadline,
        total: 0,
        entries: Vec::new(),
    };
    assert!(collect(directory.path(), &directory.parent, None, 0, &mut walk).is_err());
    assert!(walk.entries.is_empty() && file.exists());
    assert!(identity::names(directory.lock.as_ref().unwrap(), directory.path(), deadline).is_err());
    directory.cleanup().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
#[cfg_attr(windows, ignore = "requires exact pinned native cleanup cohort")]
fn fifo_inspection_is_nonblocking_and_retains_tree() {
    let parent = tempfile::tempdir().unwrap();
    let outside = parent.path().join("outside");
    std::fs::write(&outside, b"canary").unwrap();
    let mut directory = Directory::create(parent.path()).unwrap();
    let fifo = directory.path().join("fifo");
    rustix::fs::mkfifoat(
        directory.lock.as_ref().unwrap(),
        "fifo",
        rustix::fs::Mode::from_bits_truncate(0o600),
    )
    .unwrap();
    let start = Instant::now();
    assert!(directory.cleanup().is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(fifo.exists() && directory.path().exists());
    assert_eq!(std::fs::read(&outside).unwrap(), b"canary");
    assert!(open_path(&fifo).is_err());
    std::fs::remove_file(fifo).unwrap();
    directory.cleanup().unwrap();
}

#[test]
#[cfg_attr(windows, ignore = "requires exact pinned native cleanup cohort")]
fn aggregate_and_depth_bounds_precede_any_unlink() {
    let parent = tempfile::tempdir().unwrap();
    let mut directory = Directory::create(parent.path()).unwrap();
    for n in 0..9 {
        File::create(directory.path().join(format!("file{n}")))
            .unwrap()
            .set_len(MAX_FILE_BYTES)
            .unwrap();
    }
    assert!(directory.cleanup().is_err());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 9);
    for n in 0..9 {
        std::fs::remove_file(directory.path().join(format!("file{n}"))).unwrap();
    }
    let mut deep = directory.path().to_path_buf();
    for _ in 0..=MAX_DEPTH {
        deep = deep.join("d");
        std::fs::create_dir(&deep).unwrap();
    }
    assert!(directory.cleanup().is_err());
    assert!(deep.exists());
    std::fs::remove_dir(&deep).unwrap();
    directory.cleanup().unwrap();
}

#[test]
fn child_inspection_is_bound_to_retained_parent_generation() {
    let outer = tempfile::tempdir().unwrap();
    let original = outer.path().join("original");
    let other = outer.path().join("other");
    std::fs::create_dir(&original).unwrap();
    std::fs::create_dir(&other).unwrap();
    std::fs::write(original.join("leaf"), b"original").unwrap();
    std::fs::write(other.join("leaf"), b"replacement").unwrap();
    let parent = open_path(&original).unwrap();
    let expected =
        Snapshot::capture(&open_child(&parent, &original.join("leaf")).unwrap()).unwrap();
    let held = open_child(&parent, &other.join("leaf")).unwrap();
    assert!(expected.same(&Snapshot::capture(&held).unwrap()));
    drop(held);
    let moved = outer.path().join("moved");
    match std::fs::rename(&original, &moved) {
        Ok(()) => {
            std::fs::create_dir(&original).unwrap();
            std::fs::write(original.join("leaf"), b"replacement").unwrap();
            let held = open_child(&parent, &original.join("leaf")).unwrap();
            assert!(expected.same(&Snapshot::capture(&held).unwrap()));
        }
        Err(error) => {
            #[cfg(not(windows))]
            panic!("renaming the open parent failed off Windows: {error}");
            #[cfg(windows)]
            assert_eq!(error.raw_os_error(), Some(32));
        }
    }
}
