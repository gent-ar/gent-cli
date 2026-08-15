//! Versioned wire DTOs and length-prefixed JSON framing shared by every transport.

use std::io;

use gent_types::{CapabilitySet, Command, Event, HostStatus, Receipt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub protocol_min: u16,
    pub protocol_max: u16,
    #[serde(default)]
    pub capabilities: CapabilitySet,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Negotiated {
    pub protocol: u16,
    pub capabilities: CapabilitySet,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "body", rename_all = "camelCase")]
pub enum WireFrame {
    Hello(Hello),
    Negotiated(Negotiated),
    Command(Command),
    Receipt(Receipt),
    StatusRequest,
    Status(HostStatus),
    Subscribe { after_cursor: u64 },
    Events { events: Vec<Event> },
    Error { code: String, message: String },
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error(
        "protocol ranges do not overlap: client {client_min}..={client_max}, server {server_min}..={server_max}"
    )]
    IncompatibleVersion {
        client_min: u16,
        client_max: u16,
        server_min: u16,
        server_max: u16,
    },
}

/// Negotiates a shared protocol version and capability intersection.
///
/// # Errors
/// Returns [`ProtocolError::IncompatibleVersion`] when ranges do not overlap.
pub fn negotiate(
    hello: &Hello,
    server_min: u16,
    server_max: u16,
    server_capabilities: &CapabilitySet,
) -> Result<Negotiated, ProtocolError> {
    let minimum = hello.protocol_min.max(server_min);
    let maximum = hello.protocol_max.min(server_max);
    if minimum > maximum {
        return Err(ProtocolError::IncompatibleVersion {
            client_min: hello.protocol_min,
            client_max: hello.protocol_max,
            server_min,
            server_max,
        });
    }
    Ok(Negotiated {
        protocol: maximum,
        capabilities: hello.capabilities.intersection(server_capabilities),
    })
}

/// Encodes and writes one bounded length-prefixed JSON frame.
///
/// # Errors
/// Returns an I/O error when serialization or writing fails.
pub async fn write_frame<W>(writer: &mut W, frame: &WireFrame) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(frame).map_err(io::Error::other)?;
    let length = u32::try_from(body.len()).map_err(|_| io::Error::other("frame too large"))?;
    writer.write_u32(length).await?;
    writer.write_all(&body).await?;
    writer.flush().await
}

/// Reads and decodes one bounded length-prefixed JSON frame.
///
/// # Errors
/// Returns an I/O error for malformed, oversized, or incomplete frames.
pub async fn read_frame<R>(reader: &mut R) -> io::Result<WireFrame>
where
    R: AsyncRead + Unpin,
{
    let length = reader.read_u32().await?;
    let length = usize::try_from(length).map_err(|_| io::Error::other("invalid frame length"))?;
    if length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds maximum size",
        ));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::{Hello, WireFrame, negotiate, read_frame, write_frame};
    use gent_types::CapabilitySet;
    use tokio::io::duplex;

    #[test]
    fn negotiation_intersects_capabilities() {
        let hello = Hello {
            protocol_min: 1,
            protocol_max: 2,
            capabilities: CapabilitySet(vec!["events".into(), "future".into()]),
        };
        let answer = negotiate(
            &hello,
            1,
            1,
            &CapabilitySet(vec!["events".into(), "receipts".into()]),
        )
        .unwrap();
        assert_eq!(answer.protocol, 1);
        assert_eq!(answer.capabilities, CapabilitySet(vec!["events".into()]));
    }

    #[tokio::test]
    async fn framed_json_round_trips_and_ignores_additive_fields() {
        let (mut writer, mut reader) = duplex(1024);
        let frame = WireFrame::Hello(Hello {
            protocol_min: 1,
            protocol_max: 1,
            capabilities: CapabilitySet::default(),
        });
        write_frame(&mut writer, &frame).await.unwrap();
        assert_eq!(read_frame(&mut reader).await.unwrap(), frame);

        let body = br#"{"type":"hello","body":{"protocolMin":1,"protocolMax":1,"capabilities":[],"futureField":true}}"#;
        let parsed: WireFrame = serde_json::from_slice(body).unwrap();
        assert_eq!(parsed, frame);
    }
}
