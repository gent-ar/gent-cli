use gent_drivers::{
    PublicProvider,
    public_protocol::{PublicWireFact, replay_public_frames},
};
use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, TurnPhase};
use serde_json::json;

#[test]
fn codex_malformed_new_frames_do_not_poison_a_following_turn() {
    let replay = replay_public_frames(
        PublicProvider::Codex,
        &[
            json!({"method":"turn/plan/updated","params":{"plan":[{}]}}),
            json!({"method":"item/plan/delta","params":{}}),
            json!({"method":"item/completed","params":{"item":{"type":"webSearch"}}}),
            json!({"method":"turn/completed","params":{"turn":{"id":"turn-after-fault","status":"failed"}}}),
        ],
    );
    for classification in [
        "malformedCodexPlanUpdate",
        "malformedCodexThinkingDelta",
        "malformedCodexItem",
    ] {
        assert!(replay.contains(&PublicWireFact::Event(
            NormalizedProviderEvent::TransportDiagnostic {
                classification: classification.into()
            }
        )));
    }
    assert!(
        replay.contains(&PublicWireFact::Event(NormalizedProviderEvent::TurnEnded {
            turn_id: "turn-after-fault".into()
        }))
    );
    assert!(replay.contains(&PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::Failed
        }
    )));
}

#[test]
fn a_public_parser_fault_does_not_poison_the_following_provider_frame() {
    let claude = replay_public_frames(
        PublicProvider::Claude,
        &[
            json!({"type":"assistant"}),
            json!({"type":"result","is_error":false}),
        ],
    );
    assert!(claude.contains(&PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: "malformedClaudeAssistant".into()
        }
    )));
    assert!(claude.contains(&PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::Ready
        }
    )));
    let codex = replay_public_frames(
        PublicProvider::Codex,
        &[
            json!({"jsonrpc":"2.0"}),
            json!({"method":"turn/started","params":{"turn":{"id":"after-fault"}}}),
        ],
    );
    assert!(codex.contains(&PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: "malformedCodexFrame".into()
        }
    )));
    assert!(codex.contains(&PublicWireFact::Event(
        NormalizedProviderEvent::TurnStarted {
            turn_id: "after-fault".into()
        }
    )));
}
