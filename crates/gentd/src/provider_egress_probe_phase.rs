#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialProbePhase {
    Preparation,
    Busy,
    ScopeUnknown,
    StatusExecution,
    StatusProtocol,
    LoginUnavailable,
    CaptureTransport,
    CaptureProtocol,
    Transcript,
    Cleanup,
}
impl std::fmt::Display for CredentialProbePhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Preparation => "owned workspace preparation failed",
            Self::Busy => "another credential check is in progress",
            Self::ScopeUnknown => "process settlement is unproven; restart Gent before retrying",
            Self::StatusExecution => "SDK status check could not complete",
            Self::StatusProtocol => "SDK status response is invalid",
            Self::LoginUnavailable => "no supported Claude.ai login is active",
            Self::CaptureTransport => "local credential transport could not complete",
            Self::CaptureProtocol => "SDK credential response is invalid",
            Self::Transcript => "SDK created an unexpected transcript",
            Self::Cleanup => "owned probe artifacts could not be removed",
        })
    }
}
