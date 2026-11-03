//! Filesystem/signature layout units only: no runtime execution, production
//! ordinary authority, network-runtime admission, or capability seal is created.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
struct Layout {
    root: PathBuf,
    descriptor: PathBuf,
    payload: Payload,
    keys: BTreeMap<String, VerifyingKey>,
}
impl Layout {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir().canonicalize().unwrap();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = base.join(format!(
            "gent-linux-layout-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        assert_eq!(root.canonicalize().unwrap(), root);
        let mut payload = super::tests::payload();
        payload.helper.path = "runtime/linux/helper".into();
        payload.node.path = "runtime/node/bin/node".into();
        payload.gent.path = "gent".into();
        for (name, pin) in &mut payload.mcp_entries {
            pin.path = format!("mcps/{name}/dist/index.js");
        }
        for pin in [&mut payload.helper, &mut payload.node, &mut payload.gent]
            .into_iter()
            .chain(payload.mcp_entries.values_mut())
        {
            let bytes = vec![b'Q'; 128];
            pin.bytes = bytes.len() as u64;
            pin.sha256 = format!("{:x}", Sha256::digest(&bytes));
            let path = root.join(&pin.path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let descriptor = root.join("authority/linux-sandbox.json");
        std::fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
        let (bytes, keys) = super::tests::signed(payload.clone());
        std::fs::write(&descriptor, bytes).unwrap();
        Self {
            root,
            descriptor,
            payload,
            keys,
        }
    }
}
impl Drop for Layout {
    fn drop(&mut self) {
        if self.root.canonicalize().ok().as_ref() == Some(&self.root) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}
#[test]
fn actual_packaged_signature_and_path_metadata_resolve_the_same_node() {
    let layout = Layout::new();
    let node = layout.root.join("runtime/node/bin/node");
    let trusted_root = resource_root_for_node(&node).unwrap();
    assert_eq!(trusted_root, layout.root);
    let root = checked_descriptor_root(&layout.descriptor, &trusted_root).unwrap();
    let payload = verify(
        &std::fs::read(&layout.descriptor).unwrap(),
        &layout.keys,
        &"b".repeat(64),
        100,
    )
    .unwrap();
    assert_eq!(under(&root, &payload.node.path).unwrap(), node);
    assert!(layout.descriptor.starts_with(root.join("authority")));
    assert!(!node.starts_with(root.join("authority")));
}
#[test]
fn signed_descriptor_cannot_select_a_different_or_escaped_resource_root() {
    let layout = Layout::new();
    let other = Layout::new();
    assert!(
        verify(
            &std::fs::read(&other.descriptor).unwrap(),
            &layout.keys,
            &"b".repeat(64),
            100
        )
        .is_ok()
    );
    assert!(checked_descriptor_root(&other.descriptor, &layout.root).is_err());
    let mut escaped = layout.payload.clone();
    escaped.node.path = "../runtime/node/bin/node".into();
    let (bytes, keys) = super::tests::signed(escaped);
    assert!(verify(&bytes, &keys, &"b".repeat(64), 100).is_err());
    assert!(under(&layout.root, "../runtime/node/bin/node").is_err());
}
#[test]
fn packaged_descriptor_node_and_resource_aliases_fail_closed() {
    use std::os::unix::fs::symlink;
    let layout = Layout::new();
    let alias = layout.root.join("alias");
    symlink(&layout.root, &alias).unwrap();
    assert!(resource_root_for_node(&alias.join("runtime/node/bin/node")).is_err());
    assert!(
        checked_descriptor_root(&alias.join("authority/linux-sandbox.json"), &layout.root).is_err()
    );
    assert!(checked_descriptor_root(&layout.descriptor, &alias).is_err());
    let asset_alias = layout.root.join("runtime/node/bin/alias-node");
    symlink(layout.root.join("runtime/node/bin/node"), &asset_alias).unwrap();
    assert!(under(&layout.root, "runtime/node/bin/alias-node").is_err());
    let wrong_tail = layout.root.join("runtime/node/bin/not-node");
    std::fs::write(&wrong_tail, b"public metadata fixture").unwrap();
    assert_eq!(wrong_tail.canonicalize().unwrap(), wrong_tail);
    assert!(resource_root_for_node(&wrong_tail).is_err());
}
