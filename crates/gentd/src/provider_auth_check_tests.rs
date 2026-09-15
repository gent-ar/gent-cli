use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gent_protocol::ProviderAuthFrame;
use gent_types::{ProviderAuthLifecycle, ProviderAuthProvider};

use crate::{
    provider_auth_api::{ProviderAuthPort, StandaloneProviderAuthPort},
    provider_launch_budget::{ProbeRetry, warm_first_execution},
};

const RETRY: ProbeRetry = ProbeRetry {
    attempts: 2,
    delay: Duration::from_millis(1),
};

fn provider(directory: &Path) -> PathBuf {
    let path = |name: &str| directory.join(name).display().to_string();
    let executable = directory.join("provider");
    fs::write(
        &executable,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then if [ -f '{slow_version}' ]; then sleep 5; fi; echo 'provider 1.2.3'; exit 0; fi\nif [ -f '{slow_status}' ]; then sleep 5; fi\nif [ -f '{delayed}' ]; then sleep 0.4; fi\nif [ -f '{authenticated}' ]; then echo '{{\"loggedIn\":true}}'; exit 0; fi\necho '{{\"loggedIn\":false}}'; exit 1\n",
            slow_version = path("slow-version"),
            slow_status = path("slow-status"),
            delayed = path("delayed"),
            authenticated = path("authenticated"),
        ),
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    warm_first_execution(&executable);
    executable
}

fn port(directory: &Path, probe_timeout: Duration) -> StandaloneProviderAuthPort {
    StandaloneProviderAuthPort::new(crate::provider_executables::ProviderExecutables::explicit(
        Some(provider(directory)),
        None,
    ))
    .with_probe_budget(probe_timeout, RETRY, Duration::from_millis(50))
}

fn status(port: &StandaloneProviderAuthPort) -> ProviderAuthLifecycle {
    let Ok(ProviderAuthFrame::Status { status, .. }) =
        port.exchange(ProviderAuthFrame::StatusRequest {
            request_id: "status".into(),
            provider: ProviderAuthProvider::Claude,
        })
    else {
        panic!("status request must return a status");
    };
    status.lifecycle
}

fn settled(port: &StandaloneProviderAuthPort) -> ProviderAuthLifecycle {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let lifecycle = status(port);
        if lifecycle != ProviderAuthLifecycle::Checking {
            return lifecycle;
        }
        assert!(Instant::now() < deadline, "provider check never settled");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_probe_past_its_budget_reads_as_checking_and_settles_once_the_provider_answers() {
    for slowed in ["slow-version", "slow-status"] {
        let directory = tempfile::tempdir().unwrap();
        let port = port(directory.path(), Duration::from_millis(300));
        fs::write(directory.path().join(slowed), "").unwrap();

        assert_eq!(status(&port), ProviderAuthLifecycle::Checking, "{slowed}");
        assert_eq!(
            port.check_status(ProviderAuthProvider::Claude).lifecycle,
            ProviderAuthLifecycle::Checking,
            "{slowed}"
        );

        fs::remove_file(directory.path().join(slowed)).unwrap();
        assert_eq!(
            settled(&port),
            ProviderAuthLifecycle::Unauthenticated,
            "{slowed}"
        );
    }
}

#[test]
fn a_check_still_in_flight_is_checking_and_a_later_read_returns_its_settled_state() {
    let directory = tempfile::tempdir().unwrap();
    let port = port(directory.path(), Duration::from_secs(5));
    fs::write(directory.path().join("delayed"), "").unwrap();
    fs::write(directory.path().join("authenticated"), "").unwrap();

    assert_eq!(status(&port), ProviderAuthLifecycle::Checking);
    assert_eq!(settled(&port), ProviderAuthLifecycle::Authenticated);
}

#[test]
fn login_while_the_version_probe_is_past_its_budget_reports_checking() {
    let directory = tempfile::tempdir().unwrap();
    let port = port(directory.path(), Duration::from_millis(300));
    fs::write(directory.path().join("slow-version"), "").unwrap();

    let Ok(ProviderAuthFrame::Status { status, .. }) =
        port.exchange(ProviderAuthFrame::LoginRequest {
            request_id: "login".into(),
            provider: ProviderAuthProvider::Claude,
        })
    else {
        panic!("login past its probe budget must return a status");
    };
    assert_eq!(status.lifecycle, ProviderAuthLifecycle::Checking);
}

#[test]
fn an_authenticated_provider_stays_authenticated_when_a_recheck_times_out() {
    let directory = tempfile::tempdir().unwrap();
    let port = port(directory.path(), Duration::from_millis(300));
    fs::write(directory.path().join("authenticated"), "").unwrap();
    assert_eq!(settled(&port), ProviderAuthLifecycle::Authenticated);

    fs::write(directory.path().join("slow-status"), "").unwrap();
    assert_eq!(status(&port), ProviderAuthLifecycle::Authenticated);

    fs::remove_file(directory.path().join("slow-status")).unwrap();
    fs::remove_file(directory.path().join("authenticated")).unwrap();
    assert_eq!(status(&port), ProviderAuthLifecycle::Unauthenticated);
}
