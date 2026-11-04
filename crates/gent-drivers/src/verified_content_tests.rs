use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

use super::sha256;

std::thread_local! {
    pub(crate) static CONTENT_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn reads() -> usize {
    CONTENT_READS.with(std::cell::Cell::get)
}

fn age(path: &Path, seconds: u64) {
    let past = SystemTime::now() - Duration::from_secs(seconds);
    let times = fs::FileTimes::new().set_modified(past);
    #[cfg(windows)]
    let times = {
        use std::os::windows::fs::FileTimesExt;
        times.set_created(past)
    };
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(times)
        .unwrap();
}

fn settled_file(directory: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = directory.join(name);
    fs::write(&path, bytes).unwrap();
    age(&path, 60);
    path
}

#[test]
fn a_settled_unchanged_file_is_read_once_and_then_trusted_by_its_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = settled_file(directory.path(), "provider", b"first content");
    let first = sha256(&path).unwrap();
    let before = reads();
    let second = sha256(&path).unwrap();
    assert_eq!(reads(), before);
    assert_eq!(second.sha256, first.sha256);
    assert_eq!(second.canonical_path, first.canonical_path);
}

#[test]
fn a_rewritten_file_is_read_again_and_reports_its_new_content() {
    let directory = tempfile::tempdir().unwrap();
    let path = settled_file(directory.path(), "provider", b"first content");
    let first = sha256(&path).unwrap();
    fs::write(&path, b"other content").unwrap();
    age(&path, 30);
    let before = reads();
    let second = sha256(&path).unwrap();
    assert_eq!(reads(), before + 1);
    assert_ne!(second.sha256, first.sha256);
}

#[test]
fn a_file_replaced_with_equal_size_and_times_is_read_again() {
    let directory = tempfile::tempdir().unwrap();
    let path = settled_file(directory.path(), "provider", b"first content");
    let first = sha256(&path).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let replacement = directory.path().join("replacement");
    fs::write(&replacement, b"other content").unwrap();
    let times = fs::FileTimes::new().set_modified(modified);
    #[cfg(windows)]
    let times = {
        use std::os::windows::fs::FileTimesExt;
        times.set_created(fs::metadata(&path).unwrap().created().unwrap())
    };
    fs::OpenOptions::new()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(times)
        .unwrap();
    fs::rename(&replacement, &path).unwrap();
    let before = reads();
    let second = sha256(&path).unwrap();
    assert_eq!(reads(), before + 1);
    assert_ne!(second.sha256, first.sha256);
}

#[test]
fn a_recently_modified_file_is_never_trusted_from_its_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("provider");
    fs::write(&path, b"fresh content").unwrap();
    sha256(&path).unwrap();
    let before = reads();
    sha256(&path).unwrap();
    assert_eq!(reads(), before + 1);
}
