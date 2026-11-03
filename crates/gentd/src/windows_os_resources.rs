use crate::{windows_helper_inspection::inspect, windows_sandbox_runtime::PinnedSandboxHelper};
use serde::Deserialize;
use std::{
    fs::{File, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    path::PathBuf,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Report {
    schema: u32,
    nonce: String,
    system_directory: PathBuf,
    os_identity: String,
    libraries: Vec<Library>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Library {
    name: String,
    path: PathBuf,
    sha256: String,
}

/// Opaque preparation proof retaining deny-write/delete locks on each measured file.
pub(crate) struct VerifiedOsLibraries {
    system_directory: PathBuf,
    paths: Vec<PathBuf>,
    handles: Vec<File>,
}
impl VerifiedOsLibraries {
    pub(crate) fn system_directory(&self) -> &std::path::Path {
        &self.system_directory
    }
    pub(crate) fn paths(&self) -> Vec<PathBuf> {
        self.paths.clone()
    }
    pub(crate) fn into_handles(self) -> Vec<File> {
        self.handles
    }
}

fn same_windows_path(left: &std::path::Path, right: &std::path::Path) -> bool {
    left.as_os_str().eq_ignore_ascii_case(right.as_os_str())
}

fn valid_name(name: &str) -> bool {
    name.len() > 4
        && name.len() < 128
        && std::path::Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("dll"))
        && !name.contains("..")
        && name.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-')
        })
}
fn header(report: &Report, nonce: &str) -> Result<(), String> {
    if report.schema != 1
        || report.nonce != nonce
        || !report.system_directory.is_absolute()
        || !report.os_identity.starts_with("kernel32-sha256:")
        || report.os_identity.len() != 80
    {
        return Err("OS inspector identity did not match its request".into());
    }
    Ok(())
}

pub(crate) fn verify_os_libraries(
    helper: &PinnedSandboxHelper,
    names: &[String],
) -> Result<VerifiedOsLibraries, String> {
    if names.is_empty()
        || names.len() > 128
        || names.iter().any(|name| !valid_name(name))
        || names
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != names.len()
    {
        return Err("invalid finite OS dependency names".into());
    }
    let discovery_nonce = uuid::Uuid::new_v4().to_string();
    let discovered: Report = inspect(
        helper,
        "inspect-os-resources",
        std::slice::from_ref(&discovery_nonce),
    )?;
    header(&discovered, &discovery_nonce)?;
    if !discovered.libraries.is_empty() {
        return Err("OS directory discovery included unrequested files".into());
    }
    let mut handles = Vec::new();
    let mut paths = Vec::new();
    let mut hashes = Vec::new();
    for name in names {
        let path = discovered.system_directory.join(name);
        let handle = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .map_err(|_| "cannot lock exact OS dependency")?;
        let canonical =
            std::fs::canonicalize(&path).map_err(|_| "cannot canonicalize OS dependency")?;
        if !same_windows_path(&canonical, &path) {
            return Err("OS dependency resolved outside its exact system path".into());
        }
        let lock = gent_drivers::lock::capture("os-resource", &path, "1", "microsoft-protected-v1")
            .map_err(|_| "cannot measure locked OS dependency")?;
        paths.push(canonical);
        handles.push(handle);
        hashes.push(lock.digest_sha256);
    }
    let nonce = uuid::Uuid::new_v4().to_string();
    let mut arguments = vec![nonce.clone()];
    arguments.extend_from_slice(names);
    let verified: Report = inspect(helper, "inspect-os-resources", &arguments)?;
    header(&verified, &nonce)?;
    if verified.system_directory != discovered.system_directory
        || verified.os_identity != discovered.os_identity
        || verified.libraries.len() != names.len()
    {
        return Err("OS identity changed during dependency verification".into());
    }
    for (index, library) in verified.libraries.iter().enumerate() {
        if library.name != names[index]
            || !same_windows_path(&library.path, &paths[index])
            || library.sha256 != hashes[index]
        {
            return Err("OS cryptographic provenance does not match the locked dependency".into());
        }
    }
    Ok(VerifiedOsLibraries {
        system_directory: verified.system_directory,
        paths,
        handles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_finite_lowercase_dll_names_are_admitted() {
        assert!(valid_name("kernel32.dll"));
        for name in [
            "../kernel32.dll",
            "C:kernel32.dll",
            "kernel32.dll:stream",
            "KERNEL32.dll",
            "a..dll",
            "foo.exe",
            "\\server\\foo.dll",
        ] {
            assert!(!valid_name(name));
        }
    }
    #[test]
    fn replayed_or_malformed_inspection_reports_are_rejected() {
        let report = Report {
            schema: 1,
            nonce: "old".into(),
            system_directory: PathBuf::from(r"C:\Windows\System32"),
            os_identity: format!("kernel32-sha256:{}", "a".repeat(64)),
            libraries: vec![],
        };
        assert!(header(&report, "new").is_err());
    }

    #[test]
    fn locked_file_case_spelling_is_accepted() {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };
        let root = std::env::temp_dir().join(format!(
            "gent-os-case-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let original = root.join("CaseProbe.DLL");
        fs::write(&original, b"owned case probe").unwrap();
        let requested = fs::canonicalize(&root).unwrap().join("caseprobe.dll");
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&requested)
            .unwrap();
        let canonical = fs::canonicalize(&requested).unwrap();
        assert_ne!(canonical, requested);
        assert!(same_windows_path(&canonical, &requested));
        drop(held);
        fs::remove_file(original).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn native_report_case_spelling_is_accepted() {
        assert!(same_windows_path(
            std::path::Path::new(r"\\?\C:\Windows\System32\IPHLPAPI.DLL"),
            std::path::Path::new(r"\\?\C:\Windows\System32\iphlpapi.dll")
        ));
    }
    #[test]
    fn case_folding_does_not_admit_other_roots_names_or_aliases() {
        let wanted = std::path::Path::new(r"\\?\C:\Windows\System32\kernelbase.dll");
        for path in [
            r"\\?\C:\Other\System32\kernelbase.dll",
            r"\\?\C:\Windows\System32-other\kernelbase.dll",
            r"\\?\D:\Windows\System32\kernelbase.dll",
            r"\\?\C:\Windows\System32\userenv.dll",
            r"C:\Windows\System32\kernelbase.dll",
            r"\\?\C:\Windows\System32\..\System32\kernelbase.dll",
        ] {
            assert!(!same_windows_path(wanted, std::path::Path::new(path)));
        }
    }
    #[test]
    fn invalid_unicode_paths_are_not_lossily_equated() {
        use std::os::windows::ffi::OsStringExt;
        let a = std::ffi::OsString::from_wide(&[0xd800, 65]);
        let b = std::ffi::OsString::from_wide(&[0xd801, 65]);
        assert!(!same_windows_path(
            std::path::Path::new(&a),
            std::path::Path::new(&b)
        ));
    }
}
