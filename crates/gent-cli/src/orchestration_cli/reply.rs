use gent_protocol::OrchestrationFrame;

pub(super) fn valid_reply(request: &OrchestrationFrame, response: &OrchestrationFrame) -> bool {
    match (request, response) {
        (
            OrchestrationFrame::Fanout {
                request_id,
                request,
            },
            OrchestrationFrame::GraphSaved {
                request_id: response_id,
                graph,
            },
        ) => request_id == response_id && graph == &request.graph,
        (
            OrchestrationFrame::CrossReview {
                request_id,
                request,
            },
            OrchestrationFrame::GraphSaved {
                request_id: response_id,
                graph,
            },
        ) => {
            request_id == response_id
                && graph.binding.graph_id == request.graph_id
                && graph.binding.root_run_id == request.expected_parent_run_id
                && graph.host_epoch == request.host_epoch
                && graph.binding.goal_revision == request.goal_revision
                && graph.binding.policy_revision == request.policy_revision
                && graph.revision == request.expected_graph_revision.saturating_add(1)
        }
        (
            OrchestrationFrame::GraphRead {
                request_id,
                conversation_id,
                graph_id,
            },
            OrchestrationFrame::Graph {
                request_id: response_id,
                conversation_id: response_conversation_id,
                graph_id: response_graph_id,
                ..
            },
        ) => {
            request_id == response_id
                && conversation_id == response_conversation_id
                && graph_id == response_graph_id
        }
        _ => false,
    }
}
