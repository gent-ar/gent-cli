//! Protocol-only event following. The CLI never opens storage or provider processes.

use std::io::{self, Write};
use std::path::PathBuf;

use gent_protocol::{EVENT_STREAM_CAPABILITY, EventStreamFrame, read_json_frame, write_json_frame};

use crate::local_ipc::connect_and_negotiate;

/// Attaches to a local daemon stream and acknowledges output only after it reaches stdout.
pub(crate) async fn follow(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    after_cursor: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == EVENT_STREAM_CAPABILITY)
    {
        return Err("daemon did not negotiate event streaming".into());
    }
    write_json_frame(&mut stream, &EventStreamFrame::Attach { after_cursor }).await?;
    loop {
        let frame = read_json_frame::<_, EventStreamFrame>(&mut stream).await?;
        if let EventStreamFrame::Error { message, .. } = frame {
            return Err(message.into());
        }
        let acknowledgement = print_frame(&frame)?;
        if let Some(cursor) = acknowledgement {
            write_json_frame(&mut stream, &EventStreamFrame::Ack { cursor }).await?;
        }
    }
}

fn print_frame(frame: &EventStreamFrame) -> Result<Option<u64>, io::Error> {
    let cursor = acknowledged_cursor(frame);
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    serde_json::to_writer(&mut writer, frame).map_err(io::Error::other)?;
    writeln!(writer)?;
    writer.flush()?;
    Ok(cursor)
}

fn acknowledged_cursor(frame: &EventStreamFrame) -> Option<u64> {
    match frame {
        EventStreamFrame::Replay { page } | EventStreamFrame::Events { page } => {
            page.events.last().map(|event| event.cursor)
        }
        EventStreamFrame::Attach { .. }
        | EventStreamFrame::Ack { .. }
        | EventStreamFrame::Error { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::acknowledged_cursor;
    use gent_protocol::EventStreamFrame;

    #[test]
    fn acknowledgements_follow_the_last_durable_cursor() {
        let frame = EventStreamFrame::Replay {
            page: gent_types::EventPage {
                events: Vec::new(),
                next_after_cursor: None,
            },
        };
        assert_eq!(acknowledged_cursor(&frame), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn follow_requires_a_negotiated_attach_before_receiving_stream_frames() {
        use gent_protocol::{
            EVENT_STREAM_CAPABILITY, Hello, Negotiated, WireFrame, read_frame, read_json_frame,
            write_frame, write_json_frame,
        };
        use gent_types::{CapabilitySet, PROTOCOL_MAX};
        use tokio::net::UnixListener;

        use super::follow;

        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                WireFrame::Hello(Hello { capabilities, .. })
                    if capabilities.0.iter().any(|capability| capability == EVENT_STREAM_CAPABILITY)
            ));
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![EVENT_STREAM_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            assert!(matches!(
                read_json_frame::<_, EventStreamFrame>(&mut stream)
                    .await
                    .unwrap(),
                EventStreamFrame::Attach { after_cursor: 7 }
            ));
            write_json_frame(
                &mut stream,
                &EventStreamFrame::Error {
                    code: "closed".into(),
                    message: "closed".into(),
                },
            )
            .await
            .unwrap();
        });
        assert!(
            follow(Some(directory.path().into()), true, 7)
                .await
                .unwrap_err()
                .to_string()
                .contains("closed")
        );
        server.await.unwrap();
    }
}
