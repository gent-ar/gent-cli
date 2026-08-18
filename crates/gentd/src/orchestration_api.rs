//! Daemon mapping for finite provider-neutral orchestration frames.

use gent_ports::OrchestrationLedger;
use gent_protocol::OrchestrationFrame;
use gent_runtime::{OrchestrationResult, OrchestrationService};

pub(crate) fn exchange<L: OrchestrationLedger>(
    service: &OrchestrationService<L>,
    frame: OrchestrationFrame,
) -> Result<OrchestrationFrame, String> {
    match frame {
        OrchestrationFrame::Fanout {
            request_id,
            request,
        } => saved(
            service
                .fanout(&request)
                .map_err(|error| error.to_string())?,
            request_id,
        ),
        OrchestrationFrame::CrossReview {
            request_id,
            request,
        } => saved(
            service
                .cross_review(&request)
                .map_err(|error| error.to_string())?,
            request_id,
        ),
        OrchestrationFrame::GraphRead {
            request_id,
            conversation_id,
            graph_id,
        } => match service
            .graph(&graph_id)
            .map_err(|error| error.to_string())?
        {
            OrchestrationResult::Graph(graph)
                if graph.binding.conversation_id == conversation_id =>
            {
                Ok(OrchestrationFrame::Graph {
                    request_id,
                    conversation_id,
                    graph_id,
                    graph: Some(*graph),
                })
            }
            OrchestrationResult::Graph(_) | OrchestrationResult::Missing => {
                Ok(OrchestrationFrame::Graph {
                    request_id,
                    conversation_id,
                    graph_id,
                    graph: None,
                })
            }
            OrchestrationResult::DeniedObserver => {
                Err("orchestration is unavailable while gentd is observer-disabled".into())
            }
        },
        OrchestrationFrame::GraphSaved { .. } | OrchestrationFrame::Graph { .. } => {
            Err("orchestration response frames are server-only".into())
        }
    }
}

fn saved(result: OrchestrationResult, request_id: String) -> Result<OrchestrationFrame, String> {
    match result {
        OrchestrationResult::Graph(graph) => Ok(OrchestrationFrame::GraphSaved {
            request_id,
            graph: *graph,
        }),
        OrchestrationResult::DeniedObserver => {
            Err("orchestration is unavailable while gentd is observer-disabled".into())
        }
        OrchestrationResult::Missing => Err("orchestration graph is not present".into()),
    }
}
