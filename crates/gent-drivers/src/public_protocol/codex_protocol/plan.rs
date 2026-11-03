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
        .filter_map(|step| {
            let text = string(step, "step")?.trim();
            if text.is_empty() {
                return None;
            }
            Some(
                match string(step, "status").filter(|status| !status.is_empty()) {
                    Some(status) => format!("[{status}] {text}"),
                    None => text.into(),
                },
            )
        })
        .enumerate()
        .map(|(index, step)| format!("{}. {step}", index + 1))
        .collect::<Vec<_>>()
        .join("\n");
    if plan.is_empty() {
        diagnostic("malformedCodexPlanUpdate")
    } else {
        let explanation = frame
            .pointer("/params/explanation")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty());
        let text = explanation.map_or_else(|| plan.clone(), |text| format!("{text}\n\n{plan}"));
        vec![PublicWireFact::Event(
            NormalizedProviderEvent::PlanProposed { text },
        )]
    }
}

pub(super) fn proposed(item: &Value) -> Vec<PublicWireFact> {
    string(item, "text")
        .filter(|text| !text.trim().is_empty())
        .map_or_else(
            || diagnostic("malformedCodexPlanItem"),
            |text| {
                vec![PublicWireFact::Event(
                    NormalizedProviderEvent::PlanProposed { text: text.into() },
                )]
            },
        )
}

#[cfg(test)]
mod tests {
    use super::updated;
    use crate::public_protocol::PublicWireFact;
    use gent_types::NormalizedProviderEvent;
    use serde_json::json;

    #[test]
    fn preserves_plan_status_and_explanation_without_replacing_thinking() {
        assert_eq!(
            updated(&json!({"params":{"explanation":"Inspect first", "plan":[
                {"step":"Inspect the workspace","status":"completed"},
                {"step":"Implement the change","status":"inProgress"}
            ]}})),
            vec![PublicWireFact::Event(NormalizedProviderEvent::PlanProposed {
                text: "Inspect first\n\n1. [completed] Inspect the workspace\n2. [inProgress] Implement the change".into(),
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
