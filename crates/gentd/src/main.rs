//! `gentd` composition root. Product domains are assembled only behind typed ports.

mod activity_transport;
#[cfg(test)]
mod activity_transport_tests;
mod agent_chat_api;
#[cfg(test)]
mod agent_chat_api_tests;
mod agent_chat_read_transport;
mod agent_chat_subscription;
mod agent_chat_transport;
#[cfg(test)]
mod agent_chat_transport_tests;
mod agent_chat_turn_follow;
mod api;
#[allow(dead_code)]
mod approved_claude_host;
#[allow(dead_code)]
mod approved_codex_host;
#[cfg(test)]
mod approved_codex_host_bounds_tests;
mod attachment_api;
mod attachment_transport;
#[allow(dead_code)]
mod authority_clock;
#[allow(dead_code)]
mod authority_evidence_input;
mod authority_profile;
#[allow(dead_code)]
mod claude_authority_composition;
#[allow(dead_code)]
mod claude_authority_preflight;
#[allow(dead_code)]
mod claude_authority_supervisor;
#[cfg(test)]
mod claude_goal_projection_tests;
#[allow(dead_code)]
mod claude_private_resolver;
#[allow(dead_code)]
mod claude_prompt_lifecycle;
#[cfg(test)]
mod claude_prompt_lifecycle_failure_tests;
#[cfg(test)]
mod claude_prompt_lifecycle_tests;
#[allow(dead_code)]
mod claurst_acp_transport;
#[allow(dead_code)]
mod claurst_local_runtime;
#[allow(dead_code)]
mod claurst_local_runtime_owner;
#[cfg(test)]
mod claurst_local_runtime_owner_tests;
#[allow(dead_code)]
mod codex_authority_composition;
#[allow(dead_code)]
mod codex_authority_preflight;
#[allow(dead_code)]
mod codex_authority_supervisor;
#[cfg(test)]
mod codex_goal_projection_tests;
#[allow(dead_code)]
mod codex_prompt_lifecycle;
#[cfg(test)]
mod codex_prompt_lifecycle_failure_tests;
#[cfg(test)]
mod codex_prompt_lifecycle_host_tests;
#[cfg(test)]
mod codex_prompt_lifecycle_resume_tests;
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
#[allow(dead_code)]
mod fresh_compatibility_authorizer;
mod git_status_runtime;
mod goal_api;
mod goal_transport;
#[cfg(test)]
mod goal_transport_tests;
mod host_lock;
mod local_model_catalog;
#[allow(dead_code)]
mod local_model_download;
#[cfg(test)]
mod local_model_download_tests;
#[allow(dead_code)]
mod local_model_provisioning;
mod locked_provider_resolver;
#[allow(dead_code)]
mod node_runtime_lock;
mod orchestration_api;
mod orchestration_transport;
#[cfg(test)]
mod orchestration_transport_tests;
mod ordinary_authority_bootstrap;
#[allow(dead_code)]
mod ordinary_authority_composition;
#[allow(dead_code)]
mod ordinary_authority_release;
mod ordinary_lifecycle_cadence;
#[allow(dead_code)]
mod ordinary_lifecycle_control;
#[allow(dead_code)]
mod ordinary_lifecycle_router;
#[cfg(test)]
mod ordinary_lifecycle_router_tests;
mod permission_policy_api;
mod permission_policy_transport;
mod permission_workspace;
#[cfg(test)]
mod private_claurst_goal_tests;
#[allow(dead_code)]
mod private_claurst_ingress;
#[cfg(test)]
mod private_claurst_ingress_tests;
#[allow(dead_code)]
mod private_compaction_ingress;
#[cfg(test)]
mod private_compaction_ingress_tests;
#[allow(dead_code)]
mod private_lifecycle_loop;
#[cfg(unix)]
mod private_paths;
#[allow(dead_code)]
mod private_provider_compatibility;
#[allow(dead_code)]
mod private_provider_lock_validation;
#[allow(dead_code)]
mod private_provider_provisioning;
#[allow(dead_code)]
mod private_provider_provisioning_error;
#[allow(dead_code)]
mod private_provider_provisioning_sqlite;
#[allow(dead_code)]
mod private_provider_readiness;
mod private_provider_review;
#[allow(dead_code)]
mod private_provider_verifier;
#[allow(dead_code)]
mod private_provision_settlement;
#[allow(dead_code)]
mod private_session_atomic_port;
#[allow(dead_code)]
mod private_session_driver;
#[allow(dead_code)] // Only an explicit authority constructor may compose this private boundary.
mod prompt_provider_provision_boundary;
#[cfg(test)]
mod prompt_provider_provision_profile_support;
#[cfg(test)]
mod prompt_provider_provision_profile_tests;
mod prompt_provider_provision_transport;
#[allow(dead_code)] // Only an explicit authority composition may inject this admission.
mod prompt_readiness_admission;
mod provider_auth_transport;
#[cfg(test)]
mod provider_auth_transport_tests;
mod provider_effects;
#[allow(dead_code)]
mod provider_lifecycle_host;
mod provider_readiness_boundary;
mod provider_readiness_transport;
mod provider_resolver;
#[cfg(test)]
mod provider_resolver_tests;
#[allow(dead_code)]
mod public_driver_runtime;
#[cfg(test)]
mod public_driver_runtime_tests;
mod public_runs;
#[cfg(test)]
mod readiness_test_support;
mod reviewed_plan_api;
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
#[allow(dead_code)]
mod transport_shutdown;
#[cfg(all(test, unix))]
mod transport_shutdown_tests;
#[cfg(test)]
mod transport_stream_tests;
#[cfg(test)]
mod transport_tests;
#[cfg(test)]
mod transport_timeline_tests;
#[cfg(test)]
mod transport_turn_follow_tests;
#[cfg(windows)]
mod transport_windows;
#[cfg(all(test, windows))]
mod transport_windows_tests;
#[allow(dead_code)]
mod workspace_identity;
#[cfg(test)]
mod workspace_identity_tests;

#[cfg(test)]
pub(crate) use compatibility_assessment::CompatibilityAssessment;
#[cfg(test)]
pub(crate) use runtime_facade::build_runtime;
pub(crate) use runtime_facade::{RuntimeFacade, build_runtime_with_update_checks};

#[cfg(test)]
#[path = "runtime_facade_state_tests.rs"]
mod runtime_facade_state_tests;

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
