use super::{ClaurstAcpStdio, ClaurstAcpTerminal, ClaurstAcpTransport, ClaurstAcpTransportError};
use std::{collections::VecDeque, time::Instant};

struct Fake {
    reads: VecDeque<Vec<u8>>,
    exited: Option<String>,
}

impl ClaurstAcpStdio for Fake {
    fn write_frame(&mut self, _: &[u8]) -> Result<(), String> {
        Ok(())
    }
    fn try_read_frame(&mut self, _: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(self.reads.pop_front())
    }
    fn exited(&mut self) -> Result<Option<String>, String> {
        Ok(self.exited.clone())
    }
}

fn prompting() -> ClaurstAcpTransport<Fake> {
    let mut transport = ClaurstAcpTransport::new(Fake {
        reads: VecDeque::new(),
        exited: None,
    });
    transport.initialized = true;
    transport.prompt("session", "hello").unwrap();
    transport
}

#[test]
fn a_live_silent_provider_stays_pending() {
    let mut transport = prompting();
    let drain = transport.drain(8).unwrap();
    assert_eq!(drain.terminal, None);
    assert!(!transport.is_idle());
}

#[test]
fn a_provider_that_exits_mid_prompt_fails_the_drain_instead_of_going_silent() {
    let mut transport = prompting();
    transport.stdio.exited = Some("contained ACP exited: Some(125)".into());
    let Err(ClaurstAcpTransportError::Io(reason)) = transport.drain(8) else {
        panic!("an exited provider must end the turn");
    };
    assert!(reason.contains("contained ACP exited: Some(125)"));
}

#[test]
fn an_exited_provider_still_delivers_its_final_frames_first() {
    let mut transport = prompting();
    transport.stdio.exited = Some("exited".into());
    transport
        .stdio
        .reads
        .push_back(br#"{"jsonrpc":"2.0","id":1,"result":{"stopReason":"cancelled"}}"#.to_vec());
    assert_eq!(
        transport.drain(8).unwrap().terminal,
        Some(ClaurstAcpTerminal::Interrupted)
    );
}

#[test]
fn an_exited_provider_with_no_prompt_is_not_a_turn_failure() {
    let mut transport = ClaurstAcpTransport::new(Fake {
        reads: VecDeque::new(),
        exited: Some("exited".into()),
    });
    assert_eq!(transport.drain(8).unwrap().terminal, None);
}

#[test]
fn a_cancel_the_provider_never_answers_settles_interrupted_after_its_grace() {
    let mut transport = prompting();
    transport.cancel("session").unwrap();
    assert_eq!(transport.drain(8).unwrap().terminal, None);
    transport.cancel_deadline = Some(Instant::now());
    assert_eq!(
        transport.drain(8).unwrap().terminal,
        Some(ClaurstAcpTerminal::Interrupted)
    );
    assert!(transport.is_idle());
}

#[test]
fn a_new_prompt_does_not_inherit_an_earlier_cancel_deadline() {
    let mut transport = prompting();
    transport.cancel("session").unwrap();
    transport.pending_prompt_id = None;
    transport.prompt("session", "again").unwrap();
    assert_eq!(transport.cancel_deadline, None);
}
