use super::{
    CONTEXT_COMPACTION_FALLBACK_NOTICE, CONTEXT_COMPACTION_PARTIAL_NOTICE, ContextCompactionFact,
    ContextCompactionFailure, ContextCompactionPlan, ContextCompactionTrigger,
    ContextCoverageDigest,
};
use crate::{PROVIDER_CONTEXT_COMPACTED_NOTICE, PROVIDER_CONTEXT_COMPACTION_FAILED_NOTICE};

fn plan(trigger: ContextCompactionTrigger, omitted: u32) -> ContextCompactionPlan {
    ContextCompactionPlan {
        conversation_id: "conversation".into(),
        run_id: "run".into(),
        turn_id: "turn".into(),
        trigger,
        covers_through_ordinal: 4,
        covered_digest_sha256: ContextCoverageDigest::new(false).current(),
        imports_covered: false,
        previous_summary: None,
        items: Vec::new(),
        omitted_source_items: omitted,
    }
}

#[test]
fn compacted_facts_round_trip_and_refuse_unknown_fields() {
    let fact =
        plan(ContextCompactionTrigger::Command, 0).compacted("The user said LARK-7.".into(), 9);
    let value = serde_json::to_value(&fact).unwrap();
    assert_eq!(value["type"], "compacted");
    assert_eq!(value["coversThroughOrdinal"], 4);
    assert_eq!(value["trigger"], "command");
    assert_eq!(
        serde_json::from_value::<ContextCompactionFact>(value.clone()).unwrap(),
        fact
    );
    let mut forged = value;
    forged["providerSessionId"] = "native".into();
    assert!(serde_json::from_value::<ContextCompactionFact>(forged).is_err());
    assert!(fact.is_valid());
    assert_eq!(fact.event_id(), "context-compaction:turn");
}

#[test]
fn invalid_facts_are_rejected_before_they_reach_the_ledger() {
    let empty = plan(ContextCompactionTrigger::Budget, 0).compacted("  ".into(), 0);
    assert!(!empty.is_valid());
    let oversized = plan(ContextCompactionTrigger::Budget, 0)
        .compacted("s".repeat(super::MAX_CONTEXT_SUMMARY_BYTES + 1), 0);
    assert!(!oversized.is_valid());
    let mut uncovered = plan(ContextCompactionTrigger::Budget, 0);
    uncovered.covers_through_ordinal = 0;
    assert!(!uncovered.compacted("summary".into(), 1).is_valid());
}

#[test]
fn every_outcome_names_its_own_notice() {
    assert_eq!(
        plan(ContextCompactionTrigger::Budget, 0)
            .compacted("s".into(), 1)
            .notice(),
        PROVIDER_CONTEXT_COMPACTED_NOTICE
    );
    assert_eq!(
        plan(ContextCompactionTrigger::Budget, 3)
            .compacted("s".into(), 1)
            .notice(),
        CONTEXT_COMPACTION_PARTIAL_NOTICE
    );
    assert_eq!(
        plan(ContextCompactionTrigger::Command, 0)
            .failed(ContextCompactionFailure::OutputLimit)
            .notice(),
        PROVIDER_CONTEXT_COMPACTION_FAILED_NOTICE
    );
    assert_eq!(
        plan(ContextCompactionTrigger::Budget, 0)
            .failed(ContextCompactionFailure::RuntimeUnavailable)
            .notice(),
        CONTEXT_COMPACTION_FALLBACK_NOTICE
    );
}

#[test]
fn coverage_digest_depends_on_order_imports_and_every_entry() {
    let digest = |imports: bool, entries: &[(u64, &str)]| {
        let mut digest = ContextCoverageDigest::new(imports);
        for (ordinal, text) in entries {
            digest.update(*ordinal, text);
        }
        digest.current()
    };
    let base = digest(false, &[(1, "a"), (2, "b")]);
    assert_eq!(base.len(), 64);
    assert_eq!(base, digest(false, &[(1, "a"), (2, "b")]));
    assert_ne!(base, digest(true, &[(1, "a"), (2, "b")]));
    assert_ne!(base, digest(false, &[(2, "b"), (1, "a")]));
    assert_ne!(base, digest(false, &[(1, "a")]));
    assert_ne!(base, digest(false, &[(1, "a"), (3, "b")]));
}
