use std::{
    fs::File,
    io::{BufWriter, Read, Write},
    path::Path,
    time::{Duration, Instant},
};

use gent_ports::ApprovedPackageInstall;
use gent_types::ProviderInstallProgress;
use reqwest::{
    StatusCode, Url,
    blocking::{Client, Response},
    header::{ACCEPT_ENCODING, CONTENT_RANGE, RANGE},
};

use crate::installer::{InstallerError, ProviderInstallError};

const MAX_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;
const DOWNLOAD_LIMIT: Duration = Duration::from_secs(30 * 60);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

pub(super) fn download(
    package: &ApprovedPackageInstall,
    archive: &Path,
    progress: &dyn Fn(ProviderInstallProgress),
) -> Result<(), ProviderInstallError> {
    let url = archive_url(package)?;
    progress(ProviderInstallProgress::Downloading {
        received_bytes: 0,
        total_bytes: None,
    });
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(READ_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| ProviderInstallError::DownloadFailed)?;
    let response = client
        .get(url)
        .header(ACCEPT_ENCODING, "identity")
        .header(RANGE, "bytes=0-")
        .send()
        .map_err(|_| ProviderInstallError::DownloadFailed)?;
    let total = response_size(&response)?;
    let file = File::create_new(archive).map_err(|error| InstallerError::Io(error.to_string()))?;
    copy_archive(response, BufWriter::new(file), total, progress)
}

fn archive_url(package: &ApprovedPackageInstall) -> Result<Url, InstallerError> {
    let (scope, name) = package
        .package_name
        .strip_prefix('@')
        .and_then(|name| name.split_once('/'))
        .filter(|(scope, name)| [*scope, *name].into_iter().all(package_component))
        .ok_or(InstallerError::InvalidArtifact)?;
    if !package_component(&package.version) {
        return Err(InstallerError::InvalidArtifact);
    }
    Url::parse(&format!(
        "https://registry.npmjs.org/@{scope}/{name}/-/{name}-{}.tgz",
        package.version
    ))
    .map_err(|_| InstallerError::InvalidArtifact)
}

fn package_component(value: &str) -> bool {
    !value.is_empty()
        && value.starts_with(|character: char| character.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn response_size(response: &Response) -> Result<Option<u64>, ProviderInstallError> {
    let total = match response.status() {
        StatusCode::OK => response.content_length(),
        StatusCode::PARTIAL_CONTENT => {
            let range = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(full_range_size)
                .ok_or(ProviderInstallError::DownloadFailed)?;
            if response
                .content_length()
                .is_some_and(|length| length != range)
            {
                return Err(ProviderInstallError::DownloadFailed);
            }
            Some(range)
        }
        _ => return Err(ProviderInstallError::DownloadFailed),
    };
    if total.is_some_and(|value| value == 0 || value > MAX_PACKAGE_BYTES) {
        return Err(ProviderInstallError::DownloadFailed);
    }
    Ok(total)
}

fn full_range_size(value: &str) -> Option<u64> {
    let (last, total) = value.strip_prefix("bytes 0-")?.split_once('/')?;
    let last = last.parse::<u64>().ok()?;
    let total = total.parse::<u64>().ok()?;
    (last.checked_add(1)? == total).then_some(total)
}

fn copy_archive(
    mut response: impl Read,
    mut output: impl Write,
    total: Option<u64>,
    progress: &dyn Fn(ProviderInstallProgress),
) -> Result<(), ProviderInstallError> {
    let report = |received_bytes| {
        progress(ProviderInstallProgress::Downloading {
            received_bytes,
            total_bytes: total,
        })
    };
    let started = Instant::now();
    let mut last_report = started;
    let mut received = 0;
    let mut buffer = vec![0; 64 * 1024];
    report(received);
    loop {
        if started.elapsed() >= DOWNLOAD_LIMIT {
            return Err(ProviderInstallError::DownloadFailed);
        }
        let read = response
            .read(&mut buffer)
            .map_err(|_| ProviderInstallError::DownloadFailed)?;
        if read == 0 {
            break;
        }
        received += read as u64;
        if received > total.unwrap_or(MAX_PACKAGE_BYTES) {
            return Err(ProviderInstallError::DownloadFailed);
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| InstallerError::Io(error.to_string()))?;
        if last_report.elapsed() >= PROGRESS_INTERVAL {
            report(received);
            last_report = Instant::now();
        }
    }
    if received == 0 || total.is_some_and(|expected| expected != received) {
        return Err(ProviderInstallError::DownloadFailed);
    }
    output
        .flush()
        .map_err(|error| InstallerError::Io(error.to_string()))?;
    report(received);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[test]
    fn archives_are_built_from_the_exact_scoped_package_and_version() {
        let mut package = ApprovedPackageInstall {
            provider: "codex".into(),
            package_name: "@openai/codex".into(),
            version: "0.153.4-win32-x64".into(),
            integrity: String::new(),
            package_policy_digest_sha256: String::new(),
        };
        assert_eq!(
            archive_url(&package).unwrap().as_str(),
            "https://registry.npmjs.org/@openai/codex/-/codex-0.153.4-win32-x64.tgz"
        );
        package.version = "../../other".into();
        assert!(archive_url(&package).is_err());
    }

    #[test]
    fn full_range_uses_the_actual_archive_size() {
        assert_eq!(
            full_range_size("bytes 0-141495385/141495386"),
            Some(141_495_386)
        );
        assert_eq!(full_range_size("bytes 0-1/141495386"), None);
        assert_eq!(full_range_size("bytes 1-2/3"), None);
    }

    #[test]
    fn download_reports_bytes_and_rejects_a_truncated_response() {
        let events = RefCell::new(Vec::new());
        let mut archive = Vec::new();
        copy_archive(&b"tgz"[..], &mut archive, Some(3), &|value| {
            events.borrow_mut().push(value);
        })
        .unwrap();
        assert_eq!(archive, b"tgz");
        assert_eq!(
            events.into_inner(),
            [0, 3].map(|received_bytes| {
                ProviderInstallProgress::Downloading {
                    received_bytes,
                    total_bytes: Some(3),
                }
            })
        );
        assert_eq!(
            copy_archive(&b"tgz"[..], Vec::new(), Some(4), &|_| {}),
            Err(ProviderInstallError::DownloadFailed)
        );
    }
}
