use gent_types::HostEpoch;

use super::{PrivateClaudeAuthorityConfig, PrivateClaudeAuthorityError, validate};

#[test]
fn private_claude_owner_rejects_blank_or_unbounded_values() {
    for coordinator_id in [String::new(), " ".into(), "x".repeat(257)] {
        assert!(matches!(
            validate(&PrivateClaudeAuthorityConfig {
                coordinator_id,
                host_epoch: HostEpoch(1),
            }),
            Err(PrivateClaudeAuthorityError::InvalidCoordinator)
        ));
    }
}
