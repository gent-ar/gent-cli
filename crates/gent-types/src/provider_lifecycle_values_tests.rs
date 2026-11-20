use super::{NormalizedProviderEvent, ProviderFailureClassification};

fn failure(resets_at: Option<u64>) -> NormalizedProviderEvent {
    NormalizedProviderEvent::ProviderFailure {
        classification: ProviderFailureClassification::RateLimited,
        message: "Claude rate limit reached.".into(),
        resets_at,
    }
}

#[test]
fn a_known_reset_time_is_sent_as_resets_at() {
    assert_eq!(
        serde_json::to_value(failure(Some(1_763_337_600))).unwrap(),
        serde_json::json!({"type":"providerFailure","classification":"rateLimited","message":"Claude rate limit reached.","resetsAt":1_763_337_600_u64})
    );
}

#[test]
fn an_unknown_reset_time_is_absent_and_reads_back_as_none() {
    let value = serde_json::to_value(failure(None)).unwrap();
    assert!(value.get("resetsAt").is_none());
    assert_eq!(
        serde_json::from_value::<NormalizedProviderEvent>(serde_json::json!({"type":"providerFailure","classification":"rateLimited","message":"Claude rate limit reached."})).unwrap(),
        failure(None)
    );
}
