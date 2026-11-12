//! Native factory availability is distinct from a live process's enforcement evidence.
use gent_types::{AgentChatProvider, Containment};

/// Created by a managed factory after verifying its exact finite dependency set.
/// This is preparation authority, never evidence that a process was contained.
#[cfg(any(windows, target_os = "linux"))]
pub(crate) trait VerifiedProviderFactoryResources {
    #[cfg(windows)]
    fn provider(&self) -> AgentChatProvider;
    #[cfg(windows)]
    fn executable_lock(&self) -> &gent_types::RunVersionLock;
    #[cfg(windows)]
    fn command_egress(&self) -> Option<gent_types::SandboxCommandEgress>;
    fn recheck(&self) -> Result<(), String>;
}

#[cfg(windows)]
pub(crate) fn require_verified_launch(
    provider: AgentChatProvider,
    binding: &gent_types::SandboxPermissionBinding,
    lock: &gent_types::RunVersionLock,
    resources: &impl VerifiedProviderFactoryResources,
) -> Result<(), String> {
    require(provider, binding.policy().containment)?;
    require_command_contract(provider, binding.policy().containment, resources)?;
    if !binding.policy().requires_sandbox() {
        return Err("contained factory requires a supported permission binding".into());
    }
    require_matching_resources(provider, lock, resources)
}

#[cfg(windows)]
fn require_command_contract(
    provider: AgentChatProvider,
    containment: Containment,
    resources: &impl VerifiedProviderFactoryResources,
) -> Result<(), String> {
    if containment.requires_sandbox() && resources.command_egress().is_none() {
        return Err(format!(
            "{provider:?} has no brokered command environment, so its contained commands could not reach the network"
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn require_matching_resources(
    provider: AgentChatProvider,
    lock: &gent_types::RunVersionLock,
    resources: &impl VerifiedProviderFactoryResources,
) -> Result<(), String> {
    if lock.provider != provider.contained_contract().provider_id
        || resources.provider() != provider
        || resources.executable_lock() != lock
    {
        return Err("contained factory does not match the selected provider executable".into());
    }
    resources.recheck()
}

pub(crate) fn require(provider: AgentChatProvider, containment: Containment) -> Result<(), String> {
    if !containment.requires_sandbox() {
        return Ok(());
    }
    native(provider).map_err(|reason| {
        format!("The required OS sandbox is unavailable for {provider:?}: {reason}")
    })
}

pub(crate) fn sandbox_available() -> bool {
    [
        AgentChatProvider::Claude,
        AgentChatProvider::Codex,
        AgentChatProvider::Claurst,
    ]
    .into_iter()
    .all(|provider| native(provider).is_ok())
}

fn native(provider: AgentChatProvider) -> Result<(), String> {
    #[cfg(windows)]
    {
        let helper = crate::windows_sandbox_runtime::PinnedSandboxHelper::packaged()?;
        helper.require_reviewed_policy(provider)?;
        crate::windows_provider_resources::require_factory(provider)
    }
    #[cfg(not(windows))]
    {
        let _ = provider;
        Err("A sandboxed conversation requires a native containment backend that is unavailable on this host. Turn the sandbox off for this conversation.".into())
    }
}

#[cfg(all(test, windows))]
mod contained_ordinary_tests {
    use super::*;
    struct Resources(AgentChatProvider, gent_types::RunVersionLock);
    impl VerifiedProviderFactoryResources for Resources {
        fn provider(&self) -> AgentChatProvider {
            self.0
        }
        fn executable_lock(&self) -> &gent_types::RunVersionLock {
            &self.1
        }
        fn command_egress(&self) -> Option<gent_types::SandboxCommandEgress> {
            None
        }
        fn recheck(&self) -> Result<(), String> {
            Ok(())
        }
    }
    #[test]
    fn contained_launch_verification_never_admits_an_unsandboxed_conversation() {
        for (provider, name) in [
            (AgentChatProvider::Codex, "codex"),
            (AgentChatProvider::Claude, "claude"),
            (AgentChatProvider::Claurst, "claurst"),
        ] {
            let resources = Resources(
                provider,
                gent_types::RunVersionLock {
                    provider: name.into(),
                    canonical_path: "pinned".into(),
                    file_identity: "file".into(),
                    digest_sha256: "a".repeat(64),
                    version: "1".into(),
                    compatibility_entry: "entry".into(),
                },
            );
            for mode in [
                gent_types::PermissionMode::AskEveryTime,
                gent_types::PermissionMode::Bypass,
            ] {
                let binding = gent_types::SandboxPermissionBinding::new(
                    "conversation".into(),
                    "run".into(),
                    gent_types::PolicyRecord {
                        policy_id: "policy".into(),
                        workspace_id: "workspace".into(),
                        scope: gent_types::PolicyScope::ProviderPermissions,
                        revision: 1,
                        mode,
                        containment: Containment::Unsandboxed,
                        allowed_tools: vec![],
                        allowed_categories: vec![],
                    },
                    gent_types::HostEpoch(1),
                )
                .unwrap();
                assert!(
                    require_verified_launch(provider, &binding, &resources.1, &resources)
                        .is_err_and(|reason| reason.contains("supported permission binding"))
                );
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn require_verified_linux_launch(
    provider: AgentChatProvider,
    binding: &gent_types::SandboxPermissionBinding,
    lock: &gent_types::RunVersionLock,
    authority: &crate::linux_provider_factory::LinuxFactoryAuthorization,
) -> Result<(), String> {
    authority.require_launch(provider, binding, lock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_sandboxed_conversation_requires_a_provider_sandbox() {
        for provider in [
            AgentChatProvider::Claude,
            AgentChatProvider::Codex,
            AgentChatProvider::Claurst,
        ] {
            assert!(require(provider, Containment::Unsandboxed).is_ok());
        }
    }

    #[cfg(windows)]
    #[test]
    fn exact_preparation_checks_provider_executable_and_recheck_without_claiming_process_proof() {
        struct Fixture {
            lock: gent_types::RunVersionLock,
            current: bool,
            egress: Option<gent_types::SandboxCommandEgress>,
        }
        impl VerifiedProviderFactoryResources for Fixture {
            fn provider(&self) -> AgentChatProvider {
                AgentChatProvider::Claurst
            }
            fn executable_lock(&self) -> &gent_types::RunVersionLock {
                &self.lock
            }
            fn command_egress(&self) -> Option<gent_types::SandboxCommandEgress> {
                self.egress
            }
            fn recheck(&self) -> Result<(), String> {
                if self.current {
                    Ok(())
                } else {
                    Err("fixture changed".into())
                }
            }
        }
        let mut fixture = Fixture {
            lock: gent_types::RunVersionLock {
                provider: "claurst".into(),
                canonical_path: "fixture".into(),
                digest_sha256: "a".repeat(64),
                file_identity: "fixture".into(),
                version: "1".into(),
                compatibility_entry: "fixture".into(),
            },
            current: true,
            egress: None,
        };
        assert!(
            require_command_contract(AgentChatProvider::Claurst, Containment::Sandboxed, &fixture)
                .is_err_and(|reason| reason.contains("brokered command environment"))
        );
        assert!(
            require_command_contract(
                AgentChatProvider::Claurst,
                Containment::Unsandboxed,
                &fixture
            )
            .is_ok()
        );
        fixture.egress = Some(gent_types::SandboxCommandEgress::BrokeredInternet);
        assert!(
            require_command_contract(AgentChatProvider::Claurst, Containment::Sandboxed, &fixture)
                .is_ok()
        );
        assert!(
            require_matching_resources(AgentChatProvider::Claurst, &fixture.lock, &fixture).is_ok()
        );
        assert!(
            require_matching_resources(AgentChatProvider::Claude, &fixture.lock, &fixture).is_err()
        );
        let mut changed = fixture.lock.clone();
        changed.digest_sha256 = "b".repeat(64);
        assert!(
            require_matching_resources(AgentChatProvider::Claurst, &changed, &fixture).is_err()
        );
        fixture.current = false;
        assert!(
            require_matching_resources(AgentChatProvider::Claurst, &fixture.lock, &fixture)
                .is_err()
        );
    }
}
