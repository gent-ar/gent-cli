use std::io;

use crate::cli_error::{CliError, Failure};

use super::{FollowEnd, output_failed, reconnects};

#[test]
fn a_lost_transport_reconnects() {
    let lost: Box<dyn std::error::Error> = io::Error::from(io::ErrorKind::ConnectionReset).into();
    assert!(reconnects(&Err(lost)));
    let unavailable: Box<dyn std::error::Error> =
        CliError::new(Failure::Unavailable, "gentd is not running").into();
    assert!(reconnects(&Err(unavailable)));
    assert!(reconnects(&Ok(FollowEnd::ServerClosing)));
}

#[test]
fn a_typed_rejection_is_shown_instead_of_reconnecting() {
    let rejected: Box<dyn std::error::Error> =
        CliError::daemon("turnNotFound", "the turn does not exist").into();
    assert!(!reconnects(&Err(rejected)));
    let violation: Box<dyn std::error::Error> =
        "daemon delivered a non-monotonic turn cursor".into();
    assert!(!reconnects(&Err(violation)));
    assert!(!reconnects(&Err(output_failed(io::Error::from(
        io::ErrorKind::BrokenPipe
    )))));
}
