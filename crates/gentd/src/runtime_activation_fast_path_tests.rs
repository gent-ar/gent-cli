use std::fs;

use super::super::stage::tests::hashed_files;
use super::Fixture;

#[test]
fn an_unchanged_bootstrap_reactivates_without_reading_file_contents() {
    let fixture = Fixture::new("v0.2.4");
    let first = fixture.activate().unwrap();
    let before = hashed_files();
    let second = fixture.activate().unwrap();
    assert_eq!(hashed_files(), before);
    assert_eq!(second, first);
}

#[test]
fn a_changed_bootstrap_file_is_hashed_again_and_published_as_a_new_generation() {
    let fixture = Fixture::new("v0.2.4");
    let first = fixture.activate().unwrap();
    let name = super::required_files()[0];
    fs::write(fixture.bootstrap.join(name), "changed bootstrap content").unwrap();
    let before = hashed_files();
    let second = fixture.activate().unwrap();
    assert!(hashed_files() > before);
    assert_ne!(second.bootstrap_tree_sha256, first.bootstrap_tree_sha256);
    assert_ne!(second.selected_release, first.selected_release);
    assert_eq!(
        second.bootstrap_tree_sha256,
        super::super::stage::fingerprint(&fixture.bootstrap).unwrap()
    );
}

#[test]
fn a_changed_selected_release_file_is_never_trusted_and_is_replaced() {
    let fixture = Fixture::new("v0.2.4");
    let first = fixture.activate().unwrap();
    let release = fixture.root.join("releases").join(&first.selected_release);
    let name = super::required_files()[0];
    fs::write(release.join(name), "tampered release content").unwrap();
    let second = fixture.activate().unwrap();
    assert_ne!(second.selected_release, first.selected_release);
    assert_eq!(second.bootstrap_tree_sha256, first.bootstrap_tree_sha256);
    assert_eq!(
        super::super::stage::fingerprint(
            &fixture.root.join("releases").join(&second.selected_release)
        )
        .unwrap(),
        first.bootstrap_tree_sha256
    );
}

#[test]
fn a_lost_activation_record_falls_back_to_hashing_and_keeps_the_selected_generation() {
    let fixture = Fixture::new("v0.2.4");
    let first = fixture.activate().unwrap();
    fs::remove_dir_all(fixture.root.join("activation-records")).unwrap();
    let before = hashed_files();
    let second = fixture.activate().unwrap();
    assert!(hashed_files() > before);
    assert_eq!(second, first);
    let before = hashed_files();
    assert_eq!(fixture.activate().unwrap(), first);
    assert_eq!(hashed_files(), before);
}
