use gent_protocol::ProviderAuthFrame;
use gent_types::{ProviderAuthLifecycle, ProviderAuthProvider, ProviderAuthStatus};

use super::ProviderAuthPort;

pub(crate) struct FixedProviderAuth(pub(crate) ProviderAuthLifecycle);

impl ProviderAuthPort for FixedProviderAuth {
    fn exchange(&self, _: ProviderAuthFrame) -> Result<ProviderAuthFrame, String> {
        Err("unexpected auth frame".into())
    }

    fn check_status(&self, provider: ProviderAuthProvider) -> ProviderAuthStatus {
        ProviderAuthStatus {
            challenge_id: None,
            provider,
            binary_lock: None,
            lifecycle: self.0,
            selected_method: None,
            expires_at_unix_seconds: None,
        }
    }
}
