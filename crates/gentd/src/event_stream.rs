//! Read-only cursor streaming adapter. Durable resume remains the sole source of truth.

use std::io;
use std::time::Duration;

use gent_protocol::{EventStreamFrame, MAX_FRAME_BYTES, read_json_frame, write_json_frame};
use gent_types::EventPage;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::api::RuntimeApi;

const POLL_INTERVAL: Duration = Duration::from_millis(25);
const MAX_BATCH_EVENTS: usize = 64;

/// Serves exactly one negotiated stream until its client disconnects or violates the contract.
pub(crate) async fn serve<S, R>(
    stream: S,
    runtime: R,
    after_cursor: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: RuntimeApi,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut sent = send_page(&mut writer, &runtime, after_cursor, true).await?;
    let mut acknowledged = after_cursor;
    loop {
        tokio::select! {
            input = read_json_frame::<_, Value>(&mut reader) => match input {
                Ok(raw) => match serde_json::from_value(raw) {
                    Ok(EventStreamFrame::Ack { cursor }) if cursor >= acknowledged && cursor <= sent => {
                        acknowledged = cursor;
                    }
                    Ok(EventStreamFrame::Ack { .. }) => {
                        send_error(&mut writer, "invalidAck", "acknowledgement is outside the sent cursor range").await?;
                        return Ok(());
                    }
                    _ => {
                        send_error(&mut writer, "invalidStreamFrame", "only acknowledgements are valid after attach").await?;
                        return Ok(());
                    }
                },
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(error) => return Err(Box::new(error)),
            },
            () = tokio::time::sleep(POLL_INTERVAL) => {
                if acknowledged == sent {
                    sent = send_page(&mut writer, &runtime, sent, false).await?;
                }
            }
        }
    }
}

async fn send_page<W, R>(
    writer: &mut W,
    runtime: &R,
    after_cursor: u64,
    initial: bool,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>
where
    W: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    let page = runtime
        .read_event_page(after_cursor, MAX_BATCH_EVENTS)
        .map_err(io::Error::other)?;
    send_events(writer, page, after_cursor, initial).await
}

async fn send_events<W>(
    writer: &mut W,
    page: EventPage,
    after_cursor: u64,
    initial: bool,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>
where
    W: AsyncWrite + Unpin,
{
    let batches = batches(page, after_cursor)?;
    if batches.is_empty() && initial {
        write_json_frame(
            writer,
            &EventStreamFrame::Replay {
                page: EventPage {
                    events: Vec::new(),
                    next_after_cursor: None,
                },
            },
        )
        .await?;
        return Ok(after_cursor);
    }
    let mut cursor = after_cursor;
    for page in batches {
        cursor = page.events.last().map_or(cursor, |event| event.cursor);
        let frame = if initial {
            EventStreamFrame::Replay { page }
        } else {
            EventStreamFrame::Events { page }
        };
        write_json_frame(writer, &frame).await?;
    }
    Ok(cursor)
}

fn batches(page: EventPage, after_cursor: u64) -> Result<Vec<EventPage>, io::Error> {
    let mut previous = after_cursor;
    let mut result = Vec::new();
    let mut batch = Vec::new();
    for event in page.events {
        if event.cursor <= previous {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "event cursors are not ordered",
            ));
        }
        previous = event.cursor;
        batch.push(event);
        let too_large = batch.len() > MAX_BATCH_EVENTS
            || serde_json::to_vec(&EventStreamFrame::Events {
                page: EventPage {
                    events: batch.clone(),
                    next_after_cursor: None,
                },
            })
            .map_err(io::Error::other)?
            .len()
                > MAX_FRAME_BYTES;
        if too_large {
            let event = batch.pop().expect("batch contains the event just inserted");
            if batch.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "event exceeds stream frame bound",
                ));
            }
            let next_after_cursor = batch.last().map(|event| event.cursor);
            result.push(EventPage {
                events: std::mem::take(&mut batch),
                next_after_cursor,
            });
            batch.push(event);
        }
    }
    if !batch.is_empty() {
        result.push(EventPage {
            events: batch,
            next_after_cursor: page.next_after_cursor,
        });
    }
    Ok(result)
}

async fn send_error<W>(writer: &mut W, code: &str, message: &str) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    write_json_frame(
        writer,
        &EventStreamFrame::Error {
            code: code.into(),
            message: message.into(),
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::batches;
    use gent_types::{Event, EventPage, HostEpoch, ReceiptId};

    fn event(cursor: u64) -> Event {
        Event {
            cursor,
            event_id: format!("event-{cursor}"),
            receipt_id: ReceiptId(format!("receipt-{cursor}")),
            host_epoch: HostEpoch(1),
            kind: "accepted".into(),
            payload: serde_json::json!({}),
        }
    }

    #[test]
    fn batches_keep_strict_order_and_bound_count() {
        let page = EventPage {
            events: (1..=65).map(event).collect(),
            next_after_cursor: None,
        };
        let batches = batches(page, 0).unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].events.len(), 64);
        assert_eq!(batches[1].events[0].cursor, 65);
    }

    #[test]
    fn stale_or_duplicate_cursors_are_rejected() {
        assert!(
            batches(
                EventPage {
                    events: vec![event(2), event(2)],
                    next_after_cursor: None
                },
                0
            )
            .is_err()
        );
        assert!(
            batches(
                EventPage {
                    events: vec![event(1)],
                    next_after_cursor: None
                },
                1
            )
            .is_err()
        );
    }
}
