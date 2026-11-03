use super::{ClaurstAcpStdio, ClaurstAcpTransport};
use std::collections::VecDeque;

struct Recorder {
    writes: Vec<serde_json::Value>,
    reads: VecDeque<Vec<u8>>,
}

impl ClaurstAcpStdio for Recorder {
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String> {
        self.writes.push(serde_json::from_slice(frame).unwrap());
        Ok(())
    }
    fn try_read_frame(&mut self, _: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(self.reads.pop_front())
    }
}

#[test]
fn claurst_receives_the_ordinary_spelling_of_its_canonical_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().canonicalize().unwrap();
    let mut transport = ClaurstAcpTransport::new(Recorder {
        writes: vec![],
        reads: VecDeque::from([
            br#"{"jsonrpc":"2.0","id":1,"result":{"_meta":{"gentPermissionBinding":1}}}"#.to_vec(),
            br#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"acp-1"}}"#.to_vec(),
        ]),
    });
    transport.initialize_session(&canonical).unwrap();
    let cwd = transport.stdio.writes[1]["params"]["cwd"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!cwd.starts_with(r"\\?\"));
    assert_eq!(
        std::path::Path::new(&cwd).canonicalize().unwrap(),
        canonical
    );
}
