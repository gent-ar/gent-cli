use std::{cell::Cell, time::Duration};

use super::{CredentialReuse, EgressError};

#[test]
fn a_resolved_credential_is_reused_for_the_same_key_within_its_validity() {
    let reuse = CredentialReuse::new();
    let probes = Cell::new(0);
    let probe = || {
        probes.set(probes.get() + 1);
        Ok(probes.get())
    };
    let first = reuse.resolve("lock".into(), |_| Duration::from_secs(60), probe);
    let second = reuse.resolve("lock".into(), |_| Duration::from_secs(60), probe);
    let other = reuse.resolve("other".into(), |_| Duration::from_secs(60), probe);
    assert_eq!((first.unwrap(), second.unwrap(), other.unwrap()), (1, 1, 2));
}

#[test]
fn an_expired_credential_is_resolved_again() {
    let reuse = CredentialReuse::new();
    let probes = Cell::new(0);
    let probe = || {
        probes.set(probes.get() + 1);
        Ok(probes.get())
    };
    reuse
        .resolve("lock".into(), |_| Duration::ZERO, probe)
        .unwrap();
    let again = reuse.resolve("lock".into(), |_| Duration::ZERO, probe);
    assert_eq!(again.unwrap(), 2);
}

#[test]
fn a_failed_resolution_is_never_reused() {
    let reuse = CredentialReuse::<u32>::new();
    assert!(
        reuse
            .resolve(
                "lock".into(),
                |_| Duration::from_secs(60),
                || { Err(EgressError::Authentication) }
            )
            .is_err()
    );
    let next = reuse.resolve("lock".into(), |_| Duration::from_secs(60), || Ok(7));
    assert_eq!(next.unwrap(), 7);
}
