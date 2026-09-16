//! `gentd` composition root. Product domains are assembled only behind typed ports.

mod activity_transport;
mod agent_chat_api;
mod agent_chat_checkpoint_api;
mod agent_chat_checkpoint_transport;
mod agent_chat_command_transport;
mod agent_chat_conversation_config_api;
mod agent_chat_conversation_config_transport;
mod agent_chat_intent_error;
mod agent_chat_permission_api;
mod agent_chat_permission_transport;
mod agent_chat_projection_transport;
mod agent_chat_read_transport;
mod agent_chat_sessions_api;
mod agent_chat_sessions_transport;
mod agent_chat_side_question_api;
mod agent_chat_side_question_runners;
mod agent_chat_side_question_transport;
mod agent_chat_side_question_worker;
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
mod automation_api;
mod automation_transport;
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
mod claude_standalone_authority;
mod claude_summary_runner;
#[allow(dead_code)]
mod claurst_acp_bridge;
#[allow(dead_code)]
mod claurst_acp_transport;
#[allow(dead_code)]
mod claurst_local_readiness;
#[allow(dead_code)]
mod claurst_local_runtime;
#[allow(dead_code)]
mod claurst_local_runtime_owner;
#[cfg(test)]
mod claurst_local_runtime_owner_tests;
mod claurst_metadata_summary;
mod claurst_permission_policy;
mod claurst_prompt_lifecycle;
mod claurst_runtime_factory;
#[allow(dead_code)]
mod claurst_standalone_owner;
mod claurst_summary_runner;
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
#[allow(dead_code)]
mod codex_standalone_authority;
mod codex_summary_runner;
mod compatibility_assessment;
#[cfg(test)]
mod compatibility_lock_tests;
mod conversation_scoped_mcp;
mod conversation_transport;
mod daemon_bootstrap;
mod decision_mapping;
mod dependency_actions;
mod dependency_catalog;
mod event_stream;
mod forge_api;
mod forge_transport;
#[allow(dead_code)]
mod fresh_compatibility_authorizer;
mod git_status_runtime;
mod goal_api;
mod goal_pursuit_host;
mod goal_transport;
mod host_lock;
mod local_model_catalog;
#[allow(dead_code)]
mod local_model_download;
mod local_model_events;
mod local_model_integrity;
mod local_model_jobs;
#[allow(dead_code)]
mod local_model_provisioning;
mod local_model_transport;
#[allow(dead_code)]
mod local_provider_locks;
mod locked_provider_resolver;
#[allow(dead_code)]
mod node_runtime_lock;
mod orchestration_api;
mod orchestration_transport;
#[cfg(test)]
mod orchestration_transport_tests;
mod ordinary_authority_release;
#[allow(dead_code)]
mod ordinary_lifecycle_cadence;
#[allow(dead_code)]
mod ordinary_lifecycle_control;
#[allow(dead_code)]
mod ordinary_lifecycle_host;
#[allow(dead_code)]
mod ordinary_lifecycle_router;
mod packaged_claurst_runtime;
mod permission_category;
mod permission_policy_api;
mod permission_policy_transport;
mod permission_preflight;
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
mod prompt_provider_provision_boundary;
mod prompt_provider_provision_transport;
mod prompt_readiness_admission;
mod prompt_template_transport;
mod provider_attachments;
mod provider_auth_api;
mod provider_auth_transport;
#[cfg(test)]
mod provider_auth_transport_tests;
mod provider_effects;
mod provider_executables;
mod provider_launch_budget;
#[allow(dead_code)]
mod provider_lifecycle_host;
mod provider_readiness_boundary;
mod provider_readiness_transport;
mod provider_resolver;
#[allow(dead_code)]
mod public_driver_runtime;
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
mod standalone_authority_bootstrap;
#[allow(dead_code)]
mod standalone_authority_composition;
mod standalone_authority_release;
#[allow(dead_code)]
mod standalone_claurst_runtime_factory;
mod standalone_claurst_runtime_identity;
mod standalone_mcp_config;
mod standalone_provider_provision;
mod standalone_provider_readiness;
mod standalone_provider_setup;
mod startup;
mod transport;
#[allow(dead_code)]
mod transport_shutdown;
#[cfg(windows)]
mod transport_windows;
#[cfg(all(test, windows))]
mod transport_windows_tests;
mod workspace_documents;
mod workspace_documents_transport;
mod workspace_git_api;
#[cfg(test)]
mod workspace_git_api_tests;
mod workspace_git_transport;
#[allow(dead_code)]
mod workspace_identity;

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
