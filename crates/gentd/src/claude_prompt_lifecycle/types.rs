#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClaudePromptDispatchOutcome {
    Denied,
    Busy,
    Empty,
    Started { run_id: String },
    Unprovable { run_id: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ClaudePromptPoll {
    pub facts: u16,
    pub exited: bool,
}
