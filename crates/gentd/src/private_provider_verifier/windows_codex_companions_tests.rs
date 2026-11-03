use super::prepare;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "gent-companions-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("bin")).unwrap();
        fs::create_dir(root.join("codex-resources")).unwrap();
        for name in super::HELPERS {
            fs::write(root.join("codex-resources").join(name), b"approved helper").unwrap();
        }
        Self(root)
    }
    fn exe(&self) -> PathBuf {
        self.0.join("bin/codex.exe")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn exact_packaged_companions_are_pinned_and_idempotent() {
    let f = Fixture::new();
    let pins = prepare(&f.exe()).unwrap();
    assert_eq!(pins.len(), 4);
    for name in super::HELPERS {
        let alias = f.0.join("bin").join(name);
        assert_eq!(fs::read(&alias).unwrap(), b"approved helper");
        assert!(fs::OpenOptions::new().write(true).open(&alias).is_err());
    }
    drop(pins);
    assert_eq!(prepare(&f.exe()).unwrap().len(), 4);
}
#[test]
fn foreign_alias_is_preserved_and_rejected() {
    let f = Fixture::new();
    let alias = f.0.join("bin").join(super::HELPERS[0]);
    fs::write(&alias, b"foreign").unwrap();
    assert!(prepare(&f.exe()).unwrap_err().contains("differs"));
    assert_eq!(fs::read(alias).unwrap(), b"foreign");
}
#[test]
fn missing_packaged_companion_fails_without_fallback() {
    let f = Fixture::new();
    fs::remove_file(f.0.join("codex-resources").join(super::HELPERS[0])).unwrap();
    assert!(prepare(&f.exe()).is_err());
}
#[test]
fn resource_reparse_is_rejected_without_alias() {
    let f = Fixture::new();
    let source = f.0.join("codex-resources").join(super::HELPERS[0]);
    fs::remove_file(&source).unwrap();
    let outside = f.0.join("foreign");
    fs::write(&outside, b"foreign").unwrap();
    std::os::windows::fs::symlink_file(&outside, &source).unwrap();
    assert!(prepare(&f.exe()).is_err());
    assert!(!f.0.join("bin").join(super::HELPERS[0]).exists());
}
