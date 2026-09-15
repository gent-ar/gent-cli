use std::{io::Cursor, sync::mpsc};

use super::relay_acp_frames;

#[test]
fn an_over_ceiling_acp_line_becomes_a_notice_and_the_stream_continues() {
    let oversized = "x".repeat(gent_drivers::MAX_PROVIDER_FRAME_BYTES + 1);
    let stdout = Cursor::new(format!("{{\"id\":1}}\n{oversized}\n{{\"id\":2}}\n"));
    let (sender, frames) = mpsc::sync_channel(64);
    relay_acp_frames(stdout, sender);
    let frames = frames
        .try_iter()
        .map(|frame| String::from_utf8(frame.unwrap()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        frames,
        [
            r#"{"id":1}"#.to_owned(),
            format!(
                r#"{{"jsonrpc":"2.0","method":"{}"}}"#,
                crate::claurst_acp_transport::OVERSIZED_FRAME_METHOD
            ),
            r#"{"id":2}"#.to_owned(),
        ]
    );
}
