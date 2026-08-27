//! Daemon mapping for durable per-turn file checkpoint capture, listing, and restore.

use gent_ports::{AgentChatCheckpointLedger, AttachmentBlobStore};
use gent_protocol::AgentChatCheckpointFrame;
use gent_runtime::{
    AgentChatCheckpointCaptureResult, AgentChatCheckpointRestoreResult, AgentChatCheckpointService,
};
use gent_types::{
    AgentChatCheckpointCapture, AgentChatCheckpointRestore, AgentChatConversationId, HostEpoch,
};

pub(crate) fn exchange<L, B>(
    service: &AgentChatCheckpointService<L, B>,
    host_epoch: HostEpoch,
    frame: AgentChatCheckpointFrame,
) -> Result<AgentChatCheckpointFrame, String>
where
    L: AgentChatCheckpointLedger,
    B: AttachmentBlobStore,
{
    match frame {
        AgentChatCheckpointFrame::CaptureCheckpoint {
            request_id,
            receipt_id,
            conversation_id,
            run_id,
            message_ordinal,
            files,
        } => match service
            .capture(&AgentChatCheckpointCapture {
                request_id: gent_types::AgentChatRequestId(request_id.clone()),
                receipt_id: gent_types::ReceiptId(receipt_id),
                host_epoch,
                conversation_id: AgentChatConversationId(conversation_id),
                run_id: gent_types::AgentChatRunId(run_id),
                message_ordinal,
                created_at_unix_ms: unix_millis(),
                files,
            })
            .map_err(|error| error.to_string())?
        {
            AgentChatCheckpointCaptureResult::Captured(checkpoint) => {
                Ok(AgentChatCheckpointFrame::Captured {
                    request_id,
                    checkpoint,
                })
            }
            AgentChatCheckpointCaptureResult::DeniedObserver => {
                Err("agent-chat authority is disabled".into())
            }
        },
        AgentChatCheckpointFrame::ListCheckpoints {
            request_id,
            conversation_id,
        } => service
            .list(&conversation_id)
            .map(|checkpoints| AgentChatCheckpointFrame::Checkpoints {
                request_id,
                checkpoints,
            })
            .map_err(|error| error.to_string()),
        AgentChatCheckpointFrame::RestoreCheckpoint {
            request_id,
            receipt_id,
            conversation_id,
            checkpoint_id,
            restore_files,
            restore_files_confirmation,
        } => match service
            .restore(&AgentChatCheckpointRestore {
                request_id: gent_types::AgentChatRequestId(request_id.clone()),
                receipt_id: gent_types::ReceiptId(receipt_id),
                host_epoch,
                conversation_id: AgentChatConversationId(conversation_id.clone()),
                checkpoint_id,
                restore_files,
                restore_files_confirmation,
            })
            .map_err(|error| error.to_string())?
        {
            AgentChatCheckpointRestoreResult::Restored(restored) => {
                Ok(AgentChatCheckpointFrame::Restored {
                    request_id,
                    conversation_id,
                    checkpoint_id: restored.checkpoint_id,
                    run_id: restored.run_id.0,
                    visible_through_ordinal: restored.visible_through_ordinal,
                    restored_files: restored.restored_files,
                })
            }
            AgentChatCheckpointRestoreResult::DeniedObserver => {
                Err("agent-chat authority is disabled".into())
            }
        },
        AgentChatCheckpointFrame::Captured { .. }
        | AgentChatCheckpointFrame::Checkpoints { .. }
        | AgentChatCheckpointFrame::Restored { .. } => {
            Err("checkpoint response frames are server-only".into())
        }
    }
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}
