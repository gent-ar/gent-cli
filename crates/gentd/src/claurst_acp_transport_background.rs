use std::collections::BTreeSet;

use gent_types::ToolPhase;

use super::super::ClaurstAcpTransport;
use super::tool_fact;

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
            .retain(|tool| tool.phase == ToolPhase::Background);
    }

    pub(in crate::claurst_acp_transport) fn launch_background(
        &mut self,
        permission_settled: &BTreeSet<String>,
    ) {
        for tool in &mut self.open_tools {
            if tool.phase == ToolPhase::Started && !permission_settled.contains(&tool.id) {
                tool.phase = ToolPhase::Background;
                self.queued.push_back(tool_fact(
                    tool.id.clone(),
                    tool.name.clone(),
                    ToolPhase::Background,
                    None,
                    tool.input.as_ref(),
                ));
            }
        }
    }

    pub(in crate::claurst_acp_transport) fn end_background(&mut self, phase: ToolPhase) {
        let (ended, open): (Vec<_>, Vec<_>) = std::mem::take(&mut self.open_tools)
            .into_iter()
            .partition(|tool| tool.phase == ToolPhase::Background);
        self.open_tools = open;
        for tool in ended {
            self.queued.push_back(tool_fact(
                tool.id,
                tool.name,
                phase.clone(),
                None,
                tool.input.as_ref(),
            ));
        }
    }
}
