use std::collections::BTreeSet;

use gent_types::{NormalizedProviderEvent, ToolPhase, WorkPhase};
use serde_json::Value;

use super::super::{ClaurstAcpFact, ClaurstAcpTransport};
use super::tool_fact;

pub(in crate::claurst_acp_transport) fn parent_tool_call(value: &Value) -> Option<String> {
    value
        .pointer("/_meta/gentParentToolCallId")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty() && id.len() <= 256)
        .map(str::to_owned)
}

impl<S> ClaurstAcpTransport<S> {
    pub(crate) fn has_background_commands(&self) -> bool {
        self.open_tools
            .iter()
            .any(|tool| tool.phase == ToolPhase::Background)
    }

    pub(crate) fn background_sessions(&self) -> BTreeSet<String> {
        self.open_tools
            .iter()
            .filter(|tool| tool.phase == ToolPhase::Background)
            .filter_map(|tool| tool.session.clone())
            .collect()
    }

    pub(in crate::claurst_acp_transport) fn keep_background_tools(&mut self) {
        self.open_tools
            .retain(|tool| tool.phase == ToolPhase::Background || tool.parent.is_some());
    }

    pub(in crate::claurst_acp_transport) fn launch_background(
        &mut self,
        permission_settled: &BTreeSet<String>,
    ) {
        for tool in &mut self.open_tools {
            if tool.phase == ToolPhase::Started
                && tool.parent.is_none()
                && !permission_settled.contains(&tool.id)
            {
                tool.phase = ToolPhase::Background;
                self.queued.push_back(tool_fact(
                    tool.id.clone(),
                    tool.name.clone(),
                    ToolPhase::Background,
                    None,
                    tool.input.as_ref(),
                    None,
                ));
            }
        }
    }

    pub(in crate::claurst_acp_transport) fn end_background(&mut self, phase: ToolPhase) {
        let ended: Vec<_> = self
            .open_tools
            .iter()
            .filter(|tool| tool.phase == ToolPhase::Background)
            .map(|tool| tool.id.clone())
            .collect();
        for id in ended {
            self.close_child(&id, &phase);
            if let Some(index) = self.open_tools.iter().position(|tool| tool.id == id) {
                let tool = self.open_tools.remove(index);
                self.queued.push_back(tool_fact(
                    tool.id,
                    tool.name,
                    phase.clone(),
                    None,
                    tool.input.as_ref(),
                    tool.parent,
                ));
            }
        }
    }

    pub(in crate::claurst_acp_transport) fn open_child(&mut self, child: &str) {
        if self.children.insert(child.to_owned()) {
            self.queued.push_back(ClaurstAcpFact::Event(
                NormalizedProviderEvent::ChildStarted {
                    child_id: child.to_owned(),
                    parent_tool_use_id: child.to_owned(),
                },
            ));
        }
    }

    pub(in crate::claurst_acp_transport) fn close_child(&mut self, child: &str, phase: &ToolPhase) {
        if !self.children.remove(child) {
            return;
        }
        let (work, unfinished) = match phase {
            ToolPhase::Completed => (WorkPhase::Done, ToolPhase::Background),
            ToolPhase::Interrupted => (WorkPhase::Interrupted, ToolPhase::Interrupted),
            _ => (WorkPhase::Failed, ToolPhase::Failed),
        };
        let (orphans, open): (Vec<_>, Vec<_>) = std::mem::take(&mut self.open_tools)
            .into_iter()
            .partition(|tool| tool.parent.as_deref() == Some(child));
        self.open_tools = open;
        for mut tool in orphans {
            self.queued.push_back(tool_fact(
                tool.id.clone(),
                tool.name.clone(),
                unfinished.clone(),
                None,
                tool.input.as_ref(),
                tool.parent.clone(),
            ));
            if unfinished == ToolPhase::Background {
                tool.phase = ToolPhase::Background;
                self.open_tools.push(tool);
            }
        }
        self.queued.push_back(ClaurstAcpFact::Event(
            NormalizedProviderEvent::ChildTerminal {
                child_id: child.to_owned(),
                phase: work,
            },
        ));
    }
}

#[cfg(test)]
#[path = "claurst_acp_transport_child_tests.rs"]
mod child_tests;
