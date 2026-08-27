use super::{OwnedRun, ProviderProcess};
use crate::public_protocol::{PublicWireFact, claude_protocol};
use gent_types::NormalizedProviderEvent;

pub(super) fn append_terminals<P>(
    run: &mut OwnedRun<P>,
    frame: &serde_json::Value,
    facts: &mut Vec<PublicWireFact>,
) where
    P: ProviderProcess,
{
    for tool_use_id in claude_protocol::background_terminal_tool_use_ids(frame) {
        if let Some(tool_name) = run.tool_names.remove(&tool_use_id) {
            facts.extend(claude_protocol::activity(
                &tool_use_id,
                &tool_name,
                gent_types::ToolPhase::Completed,
                None,
            ));
        }
        if let Some(child_id) = run.child_ids.remove(&tool_use_id) {
            facts.push(PublicWireFact::Event(
                NormalizedProviderEvent::ChildTerminal {
                    child_id,
                    phase: gent_types::WorkPhase::Done,
                },
            ));
        }
    }
}

pub(super) fn remember_launches<P>(
    run: &mut OwnedRun<P>,
    frame: &serde_json::Value,
    facts: &mut Vec<PublicWireFact>,
) where
    P: ProviderProcess,
{
    for fact in claude_protocol::background_launches(frame) {
        if let PublicWireFact::Event(NormalizedProviderEvent::ChildStarted {
            child_id,
            parent_tool_use_id,
        }) = &fact
        {
            run.child_ids
                .entry(parent_tool_use_id.clone())
                .or_insert_with(|| child_id.clone());
        }
        facts.push(fact);
    }
}
