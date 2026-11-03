use super::*;

const FAMILY: &str = "Gent.QA.Proxy.20261029_bh4pzfjvbfcs0";
const FULL: &str = "Gent.QA.Proxy.20261029_1.0.0.0_x64__bh4pzfjvbfcs0";

#[test]
fn package_proof_matches_only_the_original_run_nonce() {
    let nonce = "b".repeat(32);
    let peer = SandboxNetworkPeer::external_package(
        FAMILY.into(),
        FULL.into(),
        nonce.clone(),
        65024,
        42,
        99,
    )
    .unwrap();
    assert!(peer.matches_active_gateway(&nonce));
    assert!(!peer.matches_active_gateway(&"c".repeat(32)));
    assert_eq!(peer.package_full_name(), Some(FULL));
    assert_eq!(peer.legacy_profile_identity(), None);
}

#[test]
fn legacy_profile_is_cleanup_identity_never_active_package_authority() {
    let nonce = "b".repeat(32);
    let peer = SandboxNetworkPeer::new(format!("GentGateway-{nonce}"), 65024, 42, 99).unwrap();
    assert!(!peer.matches_active_gateway(&nonce));
    assert!(peer.legacy_profile_identity().is_some());
    assert_eq!(peer.package_full_name(), None);
}

#[test]
fn package_family_and_full_name_must_describe_the_same_package() {
    for full in [
        "Other.1_1.0.0.0_x64__bh4pzfjvbfcs0",
        "Gent.QA.Proxy.20261029_1.0.0.0_x64__aaaaaaaaaaaaa",
        "Gent.QA.Proxy.20261029_1.0.0_x64__bh4pzfjvbfcs0",
        "Gent.QA.Proxy.20261029_1.0.0.0_x64_ ,_bh4pzfjvbfcs0",
    ] {
        assert!(
            SandboxNetworkPeer::external_package(
                FAMILY.into(),
                full.into(),
                "b".repeat(32),
                65024,
                42,
                99
            )
            .is_none(),
            "{full}"
        );
    }
}

#[test]
fn package_proof_rejects_missing_process_port_or_nonce() {
    assert!(
        SandboxNetworkPeer::external_package(
            FAMILY.into(),
            FULL.into(),
            "b".repeat(31),
            65024,
            42,
            99
        )
        .is_none()
    );
    assert!(
        SandboxNetworkPeer::external_package(FAMILY.into(), FULL.into(), "b".repeat(32), 0, 42, 99)
            .is_none()
    );
    assert!(
        SandboxNetworkPeer::external_package(
            FAMILY.into(),
            FULL.into(),
            "b".repeat(32),
            65024,
            0,
            99
        )
        .is_none()
    );
    assert!(
        SandboxNetworkPeer::external_package(
            FAMILY.into(),
            FULL.into(),
            "b".repeat(32),
            65024,
            42,
            0
        )
        .is_none()
    );
}
