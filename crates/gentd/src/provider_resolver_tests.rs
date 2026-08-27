use gent_ports::{PublicProviderResolver, PublicProviderRunError};
use gent_types::RunVersionLock;

use crate::provider_resolver::CodexOnlyResolver;

struct Resolver;

impl PublicProviderResolver for Resolver {
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        if provider == "codex" {
            Err(PublicProviderRunError::CompatibilityDenied)
        } else {
            panic!("Codex-only resolution delegated another provider")
        }
    }
}

#[test]
fn codex_only_resolver_refuses_other_providers_without_delegation() {
    let resolver = CodexOnlyResolver::new(Resolver);
    for provider in ["claude", "claurst", "gent", ""] {
        assert_eq!(
            resolver.resolve(provider),
            Err(PublicProviderRunError::CompatibilityDenied)
        );
    }
    assert_eq!(
        resolver.resolve("codex"),
        Err(PublicProviderRunError::CompatibilityDenied)
    );
}
