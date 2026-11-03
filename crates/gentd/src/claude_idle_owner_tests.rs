use gent_drivers::{claude_runner::ClaudeRunnerEffect, public_protocol::PublicWireFact};
use gent_ports::ConversationLedger;
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    CapabilitySet, DurableTurnPhase, HostEpoch, NormalizedLifecycleSignal, TurnPhase,
};

use crate::approved_claude_host::ApprovedClaudeHost;
use crate::claude_prompt_lifecycle_tests::{Resolver, Runner, compatibility, profile, prompt};
use crate::public_driver_runtime::PublicDriversRuntime;

#[test]
fn terminal_claude_session_is_idle_without_discarding_follow_up_authority() {
    for (phase, expected) in [
        (TurnPhase::Ready, DurableTurnPhase::Completed),
        (TurnPhase::Failed, DurableTurnPhase::Failed),
        (TurnPhase::Interrupted, DurableTurnPhase::Interrupted),
    ] {
        let ledger = SqliteLedger::in_memory().unwrap();
        let conversation = super::claude_conversation(&ledger);
        prompt(&ledger, &conversation, "first");
        let runner = Runner::default();
        let compatibility = compatibility();
        let runtime = PublicDriversRuntime::new(
            profile(&compatibility),
            Coordinator::new(ledger.clone(), CapabilitySet::default()),
            ledger.clone(),
            compatibility,
            runner.clone(),
            Resolver,
        )
        .unwrap();
        let mut host = ApprovedClaudeHost::new(runtime, "daemon-a".into(), HostEpoch(1), 1, None);
        host.tick().unwrap();
        assert!(host.needs_drive());
        runner.0.lock().unwrap().effects.push_back(vec![
            ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
                provider_session_id: "retained-session".into(),
            }),
            ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::RootPhase { phase },
            )),
        ]);
        host.tick().unwrap();
        assert_eq!(ledger.list_run_turns("run-a").unwrap()[0].phase, expected);
        assert!(!host.needs_drive());
        assert!(runner.0.lock().unwrap().active);
        assert!(runner.0.lock().unwrap().signals.is_empty());
        prompt(&ledger, &conversation, "follow-up");
        host.tick().unwrap();
        assert!(host.needs_drive());
        let state = runner.0.lock().unwrap();
        assert_eq!((state.starts, state.resumes), (1, 0));
        assert!(state.active);
        assert!(state.signals.is_empty());
    }
}
