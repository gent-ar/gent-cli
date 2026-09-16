use std::time::Duration;

pub(crate) const FIRST_EXECUTION_ALLOWANCE: Duration = Duration::from_secs(60);

pub(crate) const fn launch_budget(work: Duration) -> Duration {
    FIRST_EXECUTION_ALLOWANCE.saturating_add(work)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProbeRetry {
    pub(crate) attempts: u32,
    pub(crate) delay: Duration,
}

impl ProbeRetry {
    pub(crate) const BACKGROUND: Self = Self {
        attempts: 3,
        delay: Duration::from_secs(2),
    };

    pub(crate) const fn settle_bound(self, attempt: Duration) -> Duration {
        attempt
            .saturating_add(self.delay)
            .saturating_mul(self.attempts)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ProviderLaunchError {
    #[error("{0}")]
    TimedOut(String),
    #[error("{0}")]
    Failed(String),
}

impl From<String> for ProviderLaunchError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

#[cfg(all(test, unix))]
pub(crate) fn warm_first_execution(executable: &std::path::Path) {
    for _ in 0..200 {
        match std::process::Command::new(executable)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(Duration::from_millis(10));
            }
            status => {
                status.unwrap();
                return;
            }
        }
    }
    panic!("{} stayed busy after it was written", executable.display());
}
