//! Read-only discovery of a tag-bound, Sigstore-verified runtime update.

use std::process::Command;

use gent_types::{RuntimeUpdateCandidate, RuntimeVersion};
use serde::Deserialize;
use tempfile::tempdir;

const REPOSITORY: &str = "gent-ar/gent-cli";

#[derive(Debug, Deserialize)]
struct LatestRelease {
    tag_name: String,
}

#[derive(Debug, Deserialize)]
struct ArchiveManifest {
    version: String,
    target: String,
    archive: Archive,
}

#[derive(Debug, Deserialize)]
struct Archive {
    sha256: String,
}

/// Finds a verified archive digest for the host target without downloading an archive.
pub(crate) fn discover() -> Result<RuntimeUpdateCandidate, String> {
    let tag = latest_tag()?;
    let directory = tempdir().map_err(|error| error.to_string())?;
    let target = host_target().ok_or("this platform has no signed Gent release target")?;
    let name = format!("gent-{tag}-{target}.tar.gz.manifest.json");
    let manifest = directory.path().join(&name);
    let bundle = directory.path().join(format!("{name}.sigstore.json"));
    let base = release_base(&tag);
    download(&format!("{base}/{name}"), &manifest)?;
    download(&format!("{base}/{name}.sigstore.json"), &bundle)?;
    verify(&manifest, &bundle, &tag)?;
    candidate(
        &std::fs::read(&manifest).map_err(|error| error.to_string())?,
        &tag,
        target,
    )
}

fn latest_tag() -> Result<String, String> {
    let output = run(Command::new("curl").args([
        "--fail",
        "--location",
        "--silent",
        "--show-error",
        "https://api.github.com/repos/gent-ar/gent-cli/releases/latest",
    ]))?;
    let release: LatestRelease =
        serde_json::from_slice(&output).map_err(|error| error.to_string())?;
    valid_tag(&release.tag_name)
        .then_some(release.tag_name)
        .ok_or("latest release tag is invalid".into())
}

fn download(url: &str, destination: &std::path::Path) -> Result<(), String> {
    let status = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--output",
        ])
        .arg(destination)
        .arg(url)
        .status()
        .map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or("release metadata download failed".into())
}

fn verify(manifest: &std::path::Path, bundle: &std::path::Path, tag: &str) -> Result<(), String> {
    let identity =
        format!("^https://github.com/{REPOSITORY}/.github/workflows/release.yml@refs/tags/{tag}$");
    let status = Command::new("cosign")
        .args(["verify-blob", "--bundle"])
        .arg(bundle)
        .arg(manifest)
        .args([
            "--certificate-identity-regexp",
            &identity,
            "--certificate-oidc-issuer",
            "https://github.com/login/oauth",
        ])
        .status()
        .map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or("release manifest signature did not verify".into())
}

fn candidate(bytes: &[u8], tag: &str, target: &str) -> Result<RuntimeUpdateCandidate, String> {
    let manifest: ArchiveManifest =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if manifest.version != tag
        || manifest.target != target
        || !valid_digest(&manifest.archive.sha256)
    {
        return Err("signed release manifest does not match the requested archive".into());
    }
    Ok(RuntimeUpdateCandidate {
        release_version: parse_version(tag)?,
        artifact_digest_sha256: manifest.archive.sha256,
        forward_only_schema: false,
    })
}

fn release_base(tag: &str) -> String {
    std::env::var("GENT_RELEASE_BASE_URL")
        .unwrap_or_else(|_| format!("https://github.com/{REPOSITORY}/releases/download/{tag}"))
        .trim_end_matches('/')
        .to_owned()
}

fn run(command: &mut Command) -> Result<Vec<u8>, String> {
    let output = command.output().map_err(|error| error.to_string())?;
    output
        .status
        .success()
        .then_some(output.stdout)
        .ok_or("release lookup failed".into())
}

fn host_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

fn valid_tag(tag: &str) -> bool {
    parse_version(tag).is_ok()
}

fn parse_version(tag: &str) -> Result<RuntimeVersion, String> {
    let mut parts = tag
        .strip_prefix('v')
        .ok_or("release tag must start with v")?
        .split('.');
    let parse = |part: Option<&str>| -> Result<u16, String> {
        part.ok_or_else(|| "release version is incomplete".to_owned())
            .and_then(|value: &str| {
                value
                    .parse::<u16>()
                    .map_err(|_| "release version is invalid".to_owned())
            })
    };
    let version = RuntimeVersion {
        major: parse(parts.next())?,
        minor: parse(parts.next())?,
        patch: parse(parts.next())?,
    };
    parts
        .next()
        .is_none()
        .then_some(version)
        .ok_or("release version has too many parts".into())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::{candidate, host_target, parse_version};

    #[test]
    fn signed_manifest_candidate_is_target_and_digest_bound() {
        let target = host_target().unwrap();
        let manifest = format!(
            r#"{{"version":"v1.2.3","target":"{target}","archive":{{"sha256":"{}"}}}}"#,
            "a".repeat(64)
        );
        assert_eq!(
            candidate(manifest.as_bytes(), "v1.2.3", target)
                .unwrap()
                .release_version,
            parse_version("v1.2.3").unwrap()
        );
        assert!(candidate(manifest.as_bytes(), "v1.2.4", target).is_err());
    }

    #[test]
    fn version_parser_is_closed() {
        assert!(parse_version("v1.2.3").is_ok());
        assert!(parse_version("1.2.3").is_err());
        assert!(parse_version("v1.2.3-beta").is_err());
    }
}
