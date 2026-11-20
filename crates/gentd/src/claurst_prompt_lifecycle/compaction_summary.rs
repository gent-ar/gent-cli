use std::sync::Arc;

use gent_drivers::conversation_context_summary::{
    summary_chunk_bytes, summary_chunks, summary_request,
};
use gent_types::{ContextCompactionFact, ContextCompactionFailure, ContextCompactionPlan};

use crate::claurst_runtime_factory::ContextSummarizer;

pub(super) async fn summarize(
    summarizer: Option<Arc<dyn ContextSummarizer>>,
    plan: ContextCompactionPlan,
) -> ContextCompactionFact {
    let Some(summarizer) = summarizer else {
        return plan.failed(ContextCompactionFailure::RuntimeUnavailable);
    };
    let chunks = summary_chunks(
        &plan.items,
        summary_chunk_bytes(summarizer.window().summary_input_bytes),
    );
    let mut summary = plan.previous_summary.clone();
    let mut tokens = 0;
    for chunk in &chunks {
        match summarizer
            .summarize(summary_request(summary.as_deref(), chunk))
            .await
        {
            Ok(output) => {
                tokens = output.tokens;
                summary = Some(output.text);
            }
            Err(failure) => return plan.failed(failure),
        }
    }
    match summary {
        Some(summary) => plan.compacted(summary, tokens),
        None => plan.failed(ContextCompactionFailure::EmptySummary),
    }
}
