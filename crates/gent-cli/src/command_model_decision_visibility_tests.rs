use clap::Parser;

use super::Args;

#[test]
fn decision_acknowledgement_commands_are_not_public_client_actions() {
    assert!(Args::try_parse_from(["gent", "decision", "ack", "--decision-id", "d1"]).is_err());
    assert!(Args::try_parse_from(["gent", "decision", "settle", "--decision-id", "d1"]).is_err());
}
