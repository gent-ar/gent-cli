use std::process::{Command, Stdio};
use std::sync::mpsc;

use super::relay_acp_frames;

#[test]
fn an_over_ceiling_acp_line_becomes_a_notice_and_the_stream_continues() {
    let ceiling = gent_drivers::MAX_PROVIDER_FRAME_BYTES + 1;
    let mut child = Command::new("/bin/sh")
        .args([
            "-c",
            &r#"echo '{"id":1}'; head -c CEILING /dev/zero | tr '\0' x; echo; echo '{"id":2}'"#
                .replace("CEILING", &ceiling.to_string()),
        ])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, frames) = mpsc::sync_channel(64);
    relay_acp_frames(stdout, sender);
    child.wait().unwrap();
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
