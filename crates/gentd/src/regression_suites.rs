#[path = "agent_chat_transport_tests.rs"]
mod agent_chat_transport_tests;
#[path = "approved_codex_host_bounds_tests.rs"]
mod approved_codex_host_bounds_tests;
#[path = "claude_early_exit_tests.rs"]
mod claude_early_exit_tests;
#[path = "claude_failure_notice_tests.rs"]
mod claude_failure_notice_tests;
#[path = "claude_prewarm_tests.rs"]
mod claude_prewarm_tests;
#[path = "claude_warm_pool_tests.rs"]
mod claude_warm_pool_tests;
#[cfg(windows)]
#[path = "claurst_sandbox_tests.rs"]
mod claurst_sandbox_tests;
#[path = "codex_prewarm_tests.rs"]
mod codex_prewarm_tests;
#[path = "codex_warm_pool_tests.rs"]
mod codex_warm_pool_tests;
#[path = "internal_mcp_regressions.rs"]
mod internal_mcp_regressions;
#[path = "orchestration_transport_tests.rs"]
mod orchestration_transport_tests;
#[path = "provider_warm_transport_tests.rs"]
mod provider_warm_transport_tests;
#[cfg(windows)]
#[path = "real_claude_factory_smoke.rs"]
mod real_claude_factory_smoke;
#[cfg(windows)]
#[path = "real_codex_factory_smoke.rs"]
mod real_codex_factory_smoke;
#[path = "runtime_facade_chat_tests.rs"]
mod runtime_facade_chat_tests;
#[path = "runtime_facade_state_tests.rs"]
mod runtime_facade_state_tests;
