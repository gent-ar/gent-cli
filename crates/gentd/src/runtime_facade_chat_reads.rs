//! Provider-neutral chat read helpers kept apart from the runtime API implementation.

use gent_protocol::{AgentChatConversationFrame, AgentChatTranscriptFrame};

use super::RuntimeFacade;

pub(super) fn conversation(
    facade: &RuntimeFacade,
    frame: AgentChatConversationFrame,
) -> Result<AgentChatConversationFrame, String> {
    let reads = facade
        .agent_chat_reads
        .as_ref()
        .ok_or_else(|| "agent-chat conversation reads are observer-disabled".to_owned())?;
    match frame {
        AgentChatConversationFrame::SummaryRequest { conversation_id } => reads
            .summary(&conversation_id)
            .map(|summary| AgentChatConversationFrame::Summary(summary_with_mcp(facade, summary)))
            .map_err(|error| error.to_string()),
        AgentChatConversationFrame::DetailRequest { conversation_id } => reads
            .detail(&conversation_id)
            .map(|mut detail| {
                detail.summary = summary_with_mcp(facade, detail.summary);
                AgentChatConversationFrame::Detail(detail)
            })
            .map_err(|error| error.to_string()),
        AgentChatConversationFrame::Summary(_) | AgentChatConversationFrame::Detail(_) => {
            Err("agent-chat conversation response frames are server-only".into())
        }
    }
}

fn summary_with_mcp(
    facade: &RuntimeFacade,
    mut summary: gent_types::AgentChatConversationSummary,
) -> gent_types::AgentChatConversationSummary {
    summary.mcp_server_count = facade.mcp_server_count;
    summary.mcp_server_names = facade.mcp_server_names.clone();
    let git = summary
        .workspace_path
        .as_deref()
        .and_then(crate::workspace_git_api::status);
    summary.changed_file_count = git
        .as_ref()
        .map(|report| u32::try_from(report.files.len()).unwrap_or(u32::MAX));
    summary.git_branch = git.and_then(|report| report.branch);
    summary
}

pub(super) fn transcript(
    facade: &RuntimeFacade,
    frame: AgentChatTranscriptFrame,
) -> Result<AgentChatTranscriptFrame, String> {
    let reads = facade
        .agent_chat_reads
        .as_ref()
        .ok_or_else(|| "agent-chat transcript reads are observer-disabled".to_owned())?;
    match frame {
        AgentChatTranscriptFrame::PageRequest {
            conversation_id,
            after_cursor,
            limit,
        } => reads
            .transcript(&conversation_id, after_cursor, limit)
            .map(AgentChatTranscriptFrame::Page)
            .map_err(|error| error.to_string()),
        AgentChatTranscriptFrame::Page(_) => {
            Err("agent-chat transcript response frames are server-only".into())
        }
    }
}
