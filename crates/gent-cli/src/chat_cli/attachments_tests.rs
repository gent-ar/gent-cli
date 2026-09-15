use std::collections::HashMap;

use gent_protocol::{
    ATTACHMENTS_CAPABILITY, AttachmentFrame, Negotiated, WireFrame, read_frame, read_json_frame,
    write_frame, write_json_frame,
};
use gent_types::{AttachmentState, AttachmentTransfer, CapabilitySet, HostEpoch, PROTOCOL_MAX};
use tokio::net::{UnixListener, UnixStream};

use super::stage;

fn fenced(keys: &mut HashMap<String, u64>, key: &str, epoch: u64) -> Result<(), String> {
    match keys.insert(key.to_owned(), epoch) {
        Some(previous) if previous != epoch => {
            Err("idempotency key is bound to a different command".into())
        }
        _ => Ok(()),
    }
}

async fn serve_one_host_epoch(
    stream: &mut UnixStream,
    epoch: u64,
    keys: &mut HashMap<String, u64>,
) {
    let _ = read_frame(stream).await.unwrap();
    write_frame(
        stream,
        &WireFrame::Negotiated(Negotiated {
            protocol: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![ATTACHMENTS_CAPABILITY.into()]),
        }),
    )
    .await
    .unwrap();
    let mut current: Option<AttachmentTransfer> = None;
    while let Ok(frame) = read_json_frame::<_, AttachmentFrame>(stream).await {
        let outcome = match frame {
            AttachmentFrame::Begin { mut transfer } => {
                transfer.host_epoch = HostEpoch(epoch);
                fenced(keys, &transfer.idempotency_key, epoch).map(|()| transfer)
            }
            AttachmentFrame::Chunk {
                operation, offset, ..
            } => fenced(keys, &operation.idempotency_key, epoch).map(|()| {
                let mut transfer = current.clone().unwrap();
                transfer.received_bytes = offset + 5;
                transfer
            }),
            AttachmentFrame::Commit { operation } => {
                fenced(keys, &operation.idempotency_key, epoch).map(|()| {
                    let mut transfer = current.clone().unwrap();
                    transfer.state = AttachmentState::Available;
                    transfer
                })
            }
            _ => Err("unexpected attachment frame".into()),
        };
        let reply = match outcome {
            Ok(transfer) => {
                current = Some(transfer.clone());
                AttachmentFrame::Transfer { transfer }
            }
            Err(message) => AttachmentFrame::Error {
                code: "invariant".into(),
                message,
            },
        };
        write_json_frame(stream, &reply).await.unwrap();
    }
}

#[tokio::test]
async fn the_same_file_attaches_again_after_the_daemon_host_epoch_changes() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("token.txt");
    std::fs::write(&file, "hello").unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let mut keys = HashMap::new();
        for epoch in [1, 2] {
            let (mut stream, _) = listener.accept().await.unwrap();
            serve_one_host_epoch(&mut stream, epoch, &mut keys).await;
        }
    });
    let data_dir = Some(directory.path().to_path_buf());
    let first = stage(data_dir.clone(), true, std::slice::from_ref(&file))
        .await
        .unwrap();
    let second = stage(data_dir, true, std::slice::from_ref(&file))
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
}
