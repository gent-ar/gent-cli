use gent_types::RunVersionLock;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateProviderReadiness {
    Ready(RunVersionLock),
    InstallReview,
    InvalidInstallation,
    ClaurstUnavailable,
    Unavailable,
}
