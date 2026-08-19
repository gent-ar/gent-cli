use gent_types::HostEpoch;

use super::{PrivateCodexAuthorityConfig, PrivateCodexAuthorityError, validate};

#[test]
fn private_codex_owner_rejects_blank_or_unbounded_values() {
    for coordinator_id in [String::new(), " ".into(), "x".repeat(257)] {
        assert!(matches!(
            validate(&PrivateCodexAuthorityConfig {
                coordinator_id,
                host_epoch: HostEpoch(1),
            }),
            Err(PrivateCodexAuthorityError::InvalidCoordinator)
        ));
    }
}
