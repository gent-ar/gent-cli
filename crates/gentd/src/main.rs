//! `gentd` composition root. Product domains are assembled only behind typed ports.

mod activity_transport;
#[cfg(test)]
mod activity_transport_tests;
mod agent_chat_api;
mod agent_chat_read_transport;
mod agent_chat_subscription;
mod agent_chat_transport;
#[cfg(test)]
mod agent_chat_transport_tests;
mod api;
mod attachment_api;
mod attachment_transport;
mod authority_profile;
#[allow(dead_code)]
mod codex_prompt_lifecycle;
#[cfg(test)]
mod codex_prompt_lifecycle_tests;
mod compatibility_assessment;
#[cfg(test)]
mod compatibility_lock_tests;
mod conversation_transport;
mod daemon_bootstrap;
mod decision_mapping;
mod dependency_actions;
mod dependency_catalog;
#[cfg(test)]
mod dependency_catalog_tests;
mod event_stream;
mod git_status_runtime;
mod host_lock;
mod node_runtime;
mod permission_policy_api;
mod permission_policy_transport;
mod permission_workspace;
#[cfg(unix)]
mod private_paths;
mod provider_auth_transport;
#[cfg(test)]
mod provider_auth_transport_tests;
mod provider_effects;
mod provider_resolver;
#[cfg(test)]
mod provider_resolver_tests;
#[allow(dead_code)]
mod public_driver_runtime;
#[cfg(test)]
mod public_driver_runtime_tests;
mod public_runs;
mod reviewed_plan_transport;
#[cfg(test)]
mod reviewed_plan_transport_tests;
mod runtime_facade;
mod runtime_maintenance_transport;
mod runtime_update_authority;
mod runtime_update_bootstrap;
mod runtime_update_config;
mod runtime_update_recovery;
mod runtime_update_transport;
mod startup;
mod transport;
#[cfg(test)]
mod transport_decision_tests;
#[cfg(test)]
mod transport_event_tests;
#[cfg(test)]
mod transport_stream_tests;
#[cfg(test)]
mod transport_tests;
#[cfg(test)]
mod transport_timeline_tests;
#[cfg(windows)]
mod transport_windows;
#[cfg(all(test, windows))]
mod transport_windows_tests;

#[cfg(test)]
pub(crate) use compatibility_assessment::CompatibilityAssessment;
#[cfg(test)]
pub(crate) use runtime_facade::build_runtime;
pub(crate) use runtime_facade::{RuntimeFacade, build_runtime_with_update_checks};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    daemon_bootstrap::run().await
}

#[cfg(test)]
#[path = "runtime_facade_chat_tests.rs"]
mod runtime_facade_chat_tests;
#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
