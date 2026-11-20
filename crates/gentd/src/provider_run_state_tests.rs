use super::{ProviderSession, RequestPlacement, RunLiveness, RunTurns};

fn liveness(turns: &RunTurns) -> RunLiveness {
    turns.liveness(false)
}

#[test]
fn an_open_prompt_is_live_and_takes_no_other_prompt() {
    let turns = RunTurns::prompt_started();
    assert!(!liveness(&turns).idle());
    assert!(!liveness(&turns).accepts_prompt());
}

#[test]
fn a_settled_session_still_running_background_work_is_live_but_takes_a_prompt() {
    let mut turns = RunTurns::prompt_started();
    assert!(!turns.report_session(ProviderSession::Running));
    assert!(turns.close_prompt());
    assert!(!liveness(&turns).idle());
    assert!(liveness(&turns).accepts_prompt());
}

#[test]
fn a_settled_idle_session_is_idle() {
    let mut turns = RunTurns::prompt_started();
    turns.close_prompt();
    turns.report_session(ProviderSession::Idle);
    assert!(liveness(&turns).idle());
    assert!(liveness(&turns).accepts_prompt());
}

#[test]
fn pending_steers_keep_the_run_live_and_hold_the_next_prompt() {
    let turns = RunTurns::settled();
    assert!(!turns.liveness(true).idle());
    assert!(!turns.liveness(true).accepts_prompt());
}

#[test]
fn a_release_in_progress_keeps_the_run_live_until_a_new_prompt_opens() {
    let mut turns = RunTurns::settled();
    assert!(turns.begin_release());
    assert!(!turns.begin_release());
    assert!(!liveness(&turns).idle());
    assert!(!liveness(&turns).accepts_prompt());
    turns.open_prompt();
    assert!(!turns.releasing());
}

#[test]
fn a_request_belongs_to_the_root_turn_only_while_a_turn_is_open_for_the_root() {
    let mut turns = RunTurns::prompt_started();
    assert_eq!(turns.place_request(false), RequestPlacement::RootTurn);
    assert_eq!(turns.place_request(true), RequestPlacement::OutsideRootTurn);
    turns.close_prompt();
    assert_eq!(
        turns.place_request(false),
        RequestPlacement::OutsideRootTurn
    );
}

#[test]
fn closing_an_already_closed_prompt_reports_nothing_to_settle() {
    let mut turns = RunTurns::settled();
    assert!(!turns.close_prompt());
    assert!(!turns.prompt_open());
    assert!(!turns.turn_open());
}
