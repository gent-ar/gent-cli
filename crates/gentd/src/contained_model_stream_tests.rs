use super::*;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
struct CancelAfterBody {
    live: Arc<AtomicBool>,
    bytes: Vec<u8>,
}
impl io::Write for CancelAfterBody {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(b);
        if self.bytes.windows(9).any(|x| x == b"GENT-UNIT") {
            self.live.store(false, Ordering::Release);
        }
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn active_body_cancel_never_appends_success_terminal() {
    let (_, fake, request, resources) = super::tests::fixture(true, false);
    let live = Arc::new(AtomicBool::new(true));
    let check = live.clone();
    let endpoint = ModelEndpoint::start(
        Scope::new(
            fake,
            request,
            "run".into(),
            "b".repeat(64),
            "qa-model".into(),
            Arc::new(move || check.load(Ordering::Acquire)),
            resources,
        ),
        "a".repeat(32),
    )
    .unwrap();
    let mut sink = CancelAfterBody {
        live,
        bytes: Vec::new(),
    };
    let request = ModelRequest {
        method: "POST".into(),
        path: "/v1/chat/completions".into(),
        body: br#"{"model":"qa-model"}"#.to_vec(),
    };
    let error = endpoint
        .respond(request, &mut sink)
        .expect_err("cancel must fail");
    assert!(error.started);
    assert_eq!(error.error, ModelError::Revoked);
    assert!(sink.bytes.windows(9).any(|x| x == b"GENT-UNIT"));
    assert!(!sink.bytes.ends_with(b"0\r\n\r\n"));
    let until = Instant::now() + Duration::from_secs(1);
    while endpoint.revoke().is_err() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
}
