use gent_types::NormalizedProviderEvent;
use serde_json::Value;

use super::{
    PublicWireFact,
    support::{diagnostic, string},
};

pub(super) fn updated(frame: &Value) -> Vec<PublicWireFact> {
    let Some(steps) = frame.pointer("/params/plan").and_then(Value::as_array) else {
        return diagnostic("malformedCodexPlanUpdate");
    };
    let plan = steps
        .iter()
        .filter_map(|step| string(step, "step"))
        .map(str::trim)
        .filter(|step| !step.is_empty())
        .enumerate()
        .map(|(index, step)| format!("{}. {step}", index + 1))
        .collect::<Vec<_>>()
        .join("\n");
    if plan.is_empty() {
        diagnostic("malformedCodexPlanUpdate")
    } else {
        vec![PublicWireFact::Event(NormalizedProviderEvent::Thinking {
            text: plan,
            is_partial: false,
        })]
    }
}

#[cfg(test)]
mod tests {
    use super::updated;
    use crate::public_protocol::PublicWireFact;
    use gent_types::NormalizedProviderEvent;
    use serde_json::json;

    #[test]
    fn preserves_a_completed_provider_plan_as_final_normalized_thinking() {
        assert_eq!(
            updated(&json!({"params":{"plan":[
                {"step":"Inspect the workspace","status":"completed"},
                {"step":"Implement the change","status":"inProgress"}
            ]}})),
            vec![PublicWireFact::Event(NormalizedProviderEvent::Thinking {
                text: "1. Inspect the workspace\n2. Implement the change".into(),
                is_partial: false,
            })]
        );
    }

    #[test]
    fn rejects_an_empty_or_malformed_provider_plan() {
        for frame in [json!({"params":{"plan":[]}}), json!({"params":{}})] {
            assert!(matches!(
                updated(&frame).as_slice(),
                [PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic { classification })]
                    if classification == "malformedCodexPlanUpdate"
            ));
        }
    }
}
