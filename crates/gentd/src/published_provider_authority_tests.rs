use std::{fs, io::Write, path::Path};

use ed25519_dalek::SigningKey;

use super::{PublishedAuthorityError, PublishedAuthorityOutcome, PublishedProviderAuthority};
use crate::{
    node_runtime_lock::AppNodeRuntimeLock,
    ordinary_authority_release::{OrdinaryAuthorityReleaseError, fixture},
    standalone_authority_release::StandaloneAuthorityRelease,
};

const ROOT: [u8; 32] = [21; 32];

struct Install {
    _root: tempfile::TempDir,
    packaged: std::path::PathBuf,
    runtime: AppNodeRuntimeLock,
}

impl Install {
    fn new(packaged_expiry: u64) -> Self {
        let root = tempfile::tempdir().unwrap();
        let runtime = fixture::runtime(root.path());
        let packaged = root.path().join("release/ordinary-authority.json");
        fs::create_dir_all(packaged.parent().unwrap()).unwrap();
        let install = Self {
            _root: root,
            packaged,
            runtime,
        };
        fs::write(&install.packaged, install.document(&ROOT, packaged_expiry)).unwrap();
        install
    }

    fn document(&self, signer: &[u8; 32], expiry: u64) -> Vec<u8> {
        let release = fixture::release_expiring(
            &SigningKey::from_bytes(signer),
            self.runtime.node_digest_sha256(),
            expiry,
        );
        serde_json::to_vec(&release).unwrap()
    }

    fn start(&self) -> StandaloneAuthorityRelease {
        let key = SigningKey::from_bytes(&ROOT).verifying_key();
        StandaloneAuthorityRelease::configured(
            self.packaged.clone(),
            &[format!("root:{}", hex::encode(key.as_bytes()))],
            self.runtime.clone(),
        )
        .unwrap()
    }

    fn directory(&self) -> std::path::PathBuf {
        self.packaged.parent().unwrap().join("provider-authority")
    }
}

fn active_expiry(release: &StandaloneAuthorityRelease) -> u64 {
    release.load(now()).unwrap().expires_at_unix_seconds()
}

fn now() -> u64 {
    crate::startup::unix_seconds()
}

fn serve(body: Vec<u8>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://{}/ordinary-authority.json",
        listener.local_addr().unwrap()
    );
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        std::io::Read::read(&mut stream, &mut request).unwrap();
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
    });
    url
}

fn publish(
    install: &Install,
    body: Vec<u8>,
) -> Result<PublishedAuthorityOutcome, PublishedAuthorityError> {
    PublishedProviderAuthority::new(install.start(), serve(body)).update()
}

#[test]
fn a_newer_signed_document_is_staged_and_adopted_at_the_next_start() {
    let install = Install::new(now() + 1_000);
    let running = install.start();
    let newer = install.document(&ROOT, now() + 2_000);
    let outcome = PublishedProviderAuthority::new(running.clone(), serve(newer)).update();
    assert_eq!(outcome.unwrap(), PublishedAuthorityOutcome::Staged);
    assert_eq!(running.path(), install.packaged);
    assert!(active_expiry(&running) < now() + 2_000);
    let next = install.start();
    assert_eq!(
        next.path(),
        install.directory().join("ordinary-authority.json")
    );
    assert!(active_expiry(&next) > now() + 1_000);
    assert!(!install.directory().join("published.json").exists());
}

#[test]
fn an_untrusted_or_altered_document_is_refused_and_nothing_is_staged() {
    let install = Install::new(now() + 1_000);
    let foreign = install.document(&[22; 32], now() + 2_000);
    assert!(matches!(
        publish(&install, foreign),
        Err(PublishedAuthorityError::Refused(
            OrdinaryAuthorityReleaseError::InvalidSignature
        ))
    ));
    let mut altered: serde_json::Value =
        serde_json::from_slice(&install.document(&ROOT, now() + 2_000)).unwrap();
    altered["payload"]["expires_at_unix_seconds"] = (now() + 3_000).into();
    assert!(matches!(
        publish(&install, serde_json::to_vec(&altered).unwrap()),
        Err(PublishedAuthorityError::Refused(
            OrdinaryAuthorityReleaseError::InvalidSignature
        ))
    ));
    assert!(!install.directory().join("published.json").exists());
    assert_eq!(install.start().path(), install.packaged);
}

#[test]
fn a_document_not_newer_than_the_active_or_staged_one_is_not_staged() {
    let install = Install::new(now() + 2_000);
    let older = install.document(&ROOT, now() + 1_000);
    assert_eq!(
        publish(&install, older).unwrap(),
        PublishedAuthorityOutcome::Current
    );
    assert!(!install.directory().join("published.json").exists());
    let newer = install.document(&ROOT, now() + 3_000);
    assert_eq!(
        publish(&install, newer.clone()).unwrap(),
        PublishedAuthorityOutcome::Staged
    );
    assert_eq!(
        publish(&install, newer).unwrap(),
        PublishedAuthorityOutcome::Current
    );
}

#[test]
fn a_newer_packaged_authority_wins_over_an_older_adopted_one() {
    let install = Install::new(now() + 3_000);
    fs::create_dir_all(install.directory()).unwrap();
    fs::write(
        install.directory().join("ordinary-authority.json"),
        install.document(&ROOT, now() + 2_000),
    )
    .unwrap();
    assert_eq!(install.start().path(), install.packaged);
}

#[test]
fn an_adopted_document_that_no_longer_verifies_falls_back_to_the_packaged_one() {
    let install = Install::new(now() + 1_000);
    fs::create_dir_all(install.directory()).unwrap();
    let other_node = fixture::release_expiring(
        &SigningKey::from_bytes(&ROOT),
        &"f".repeat(64),
        now() + 2_000,
    );
    fs::write(
        install.directory().join("ordinary-authority.json"),
        serde_json::to_vec(&other_node).unwrap(),
    )
    .unwrap();
    assert_eq!(install.start().path(), install.packaged);
}

#[test]
fn an_unreachable_publisher_is_a_typed_fetch_error_and_launch_keeps_its_authority() {
    let install = Install::new(now() + 1_000);
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://{}/ordinary-authority.json",
        closed.local_addr().unwrap()
    );
    drop(closed);
    let running = install.start();
    assert!(matches!(
        PublishedProviderAuthority::new(running.clone(), url).update(),
        Err(PublishedAuthorityError::Fetch(_))
    ));
    assert!(running.load(now()).unwrap().authorizes_provider("codex"));
    assert!(!Path::new(&install.directory()).exists());
}
