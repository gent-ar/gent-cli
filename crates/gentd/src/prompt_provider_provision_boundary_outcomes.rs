use std::sync::{Arc, Mutex};

use gent_ports::AgentChatPromptDispatchLedger;
use gent_protocol::{PromptProviderProvisionFrame, PromptProviderProvisionState};

use super::{Outcome, boundary, confirm, plan_digest, seeded};

#[test]
fn ambiguous_effect_is_unprovable_and_an_exact_retry_never_replays_it() {
    let (ledger, saved) = seeded();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let boundary = boundary(ledger, Outcome::Ambiguous, Arc::clone(&calls));
    let request = confirm(&saved, true, plan_digest());
    for _ in 0..2 {
        let reply = boundary.confirm(request.clone()).unwrap();
        assert!(matches!(
            reply,
            PromptProviderProvisionFrame::Result {
                state: PromptProviderProvisionState::Unprovable,
                ..
            }
        ));
    }
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[test]
fn proven_pre_effect_failure_is_retryable_without_replaying_npm() {
    let (ledger, saved) = seeded();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let boundary = boundary(
        ledger.clone(),
        Outcome::PreEffectFailure,
        Arc::clone(&calls),
    );
    let reply = boundary
        .confirm(confirm(&saved, true, plan_digest()))
        .unwrap();
    assert!(matches!(
        reply,
        PromptProviderProvisionFrame::Result {
            state: PromptProviderProvisionState::Failed,
            ..
        }
    ));
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch(
                "test",
                gent_types::HostEpoch(1),
                gent_types::AgentChatProvider::Codex
            )
            .unwrap()
            .is_none()
    );
}
