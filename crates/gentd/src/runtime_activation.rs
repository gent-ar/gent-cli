use std::{fs, path::Path};

use serde::Deserialize;

#[cfg(test)]
use std::path::PathBuf;

#[path = "runtime_activation_command.rs"]
#[cfg(test)]
mod command;
#[cfg(test)]
pub(crate) use command::{RuntimeCommand, execute};
#[path = "runtime_activation_identity.rs"]
mod identity;
use identity::bootstrap_identity;
#[path = "runtime_activation_files.rs"]
mod files;
use files::required_files;
#[path = "runtime_activation_path.rs"]
mod generation_path;
#[path = "runtime_activation_record.rs"]
mod record;
#[path = "runtime_activation_stage.rs"]
mod stage;
#[path = "runtime_activation_version.rs"]
mod version;
use version::{is_generation, valid_version, version_of};
#[path = "runtime_activation_lease.rs"]
mod lease;
#[path = "runtime_activation_selection.rs"]
mod selection;
#[cfg(test)]
use selection::active_daemon;
use selection::{refresh_auto_update_helper, select_release, selected_release};
#[path = "runtime_activation_scheduling.rs"]
mod scheduling;
#[cfg(all(test, not(windows)))]
use scheduling::active_cli;
#[cfg(test)]
use scheduling::default_root;
use scheduling::enable_updates;
#[cfg(test)]
use scheduling::{auto_update_helper_name, enable_update_arguments};

#[derive(Deserialize)]
struct Bootstrap {
    version: String,
    target: String,
}

use gent_protocol::runtime_activation::ActivationResult;
#[path = "runtime_activation_operation.rs"]
pub(crate) mod operation;
#[cfg(test)]
pub(crate) fn activate(
    bootstrap: PathBuf,
    root: Option<PathBuf>,
    data_dir: PathBuf,
) -> Result<PathBuf, String> {
    activate_expected(bootstrap, root, data_dir, None, None, None).map(|result| result.daemon_path)
}

#[cfg(all(test, not(windows)))]
fn activate_at(
    bootstrap: &Path,
    root: &Path,
    data_dir: &Path,
    scheduler: fn(&Path, &Path) -> Result<(), String>,
) -> Result<PathBuf, String> {
    activate_at_expected(bootstrap, root, data_dir, scheduler, None, None, None)
        .map(|result| result.daemon_path)
}

#[cfg(test)]
fn activate_expected(
    bootstrap: PathBuf,
    root: Option<PathBuf>,
    data_dir: PathBuf,
    expected_digest: Option<&str>,
    expected_epoch: Option<u64>,
    expected_tree: Option<&str>,
) -> Result<ActivationResult, String> {
    match root {
        Some(root) => activate_at_expected(
            &bootstrap,
            &root,
            &data_dir,
            |_, _| Ok(()),
            expected_digest,
            expected_epoch,
            expected_tree,
        ),
        None => activate_at_expected(
            &bootstrap,
            &default_root(),
            &data_dir,
            enable_updates,
            expected_digest,
            expected_epoch,
            expected_tree,
        ),
    }
}

fn activate_at_expected(
    bootstrap: &Path,
    root: &Path,
    data_dir: &Path,
    scheduler: fn(&Path, &Path) -> Result<(), String>,
    expected_digest: Option<&str>,
    expected_epoch: Option<u64>,
    expected_tree: Option<&str>,
) -> Result<ActivationResult, String> {
    if [expected_digest, expected_tree]
        .into_iter()
        .flatten()
        .any(|digest| digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err("planned daemon digest is invalid".into());
    }
    let bootstrap = bootstrap.canonicalize().map_err(display)?;
    let held = lease::ActivationLease::acquire(root, data_dir, expected_epoch)?;
    let root = &held.root;
    if root.starts_with(&bootstrap) || held.data_dir.starts_with(&bootstrap) {
        return Err("runtime store and owner data cannot be inside the bootstrap source".into());
    }
    let descriptor: Bootstrap =
        serde_json::from_slice(&fs::read(bootstrap.join("bootstrap.json")).map_err(display)?)
            .map_err(display)?;
    let target = expected_target()?;
    if descriptor.target != target || !valid_version(&descriptor.version) {
        return Err("Gent bootstrap metadata does not match this platform".into());
    }
    verify_bootstrap(&bootstrap)?;
    let current = selected_release(root)?;
    if current
        .as_ref()
        .is_some_and(|current| version_of(current) > version_of(&descriptor.version))
    {
        if expected_digest.is_some() || expected_tree.is_some() {
            return Err("a newer runtime was selected before repair activation".into());
        }
        let current = current.ok_or("selected runtime missing")?;
        return match record::release(root, &current) {
            Some(known) => result_known(root, &held, current, &known),
            None => result(root, &held, current, None),
        };
    }
    let same_version = current.filter(|current| {
        is_generation(current, &target) && version_of(current) == version_of(&descriptor.version)
    });
    let bootstrap_stat = stage::stat(&bootstrap)?;
    if let Some(known) = same_version
        .as_deref()
        .and_then(|current| record::unchanged(root, current, &bootstrap, &bootstrap_stat))
    {
        if expected_digest.is_none_or(|expected| known.executable.eq_ignore_ascii_case(expected))
            && expected_tree.is_none_or(|expected| known.tree.eq_ignore_ascii_case(expected))
        {
            let selected = same_version.ok_or("selected runtime missing")?;
            let activated = result_known(root, &held, selected, &known)?;
            schedule(scheduler, root, &held.data_dir);
            return Ok(activated);
        }
    }
    let tree = stage::scan(&bootstrap)?;
    let known = record::Known {
        executable: tree.digest(daemon_name())?,
        tree: tree.fingerprint.clone(),
    };
    if expected_digest.is_some_and(|expected| !known.executable.eq_ignore_ascii_case(expected)) {
        return Err("prepared daemon no longer matches the planned executable digest".into());
    }
    if expected_tree.is_some_and(|expected| !known.tree.eq_ignore_ascii_case(expected)) {
        return Err("prepared runtime tree no longer matches the verified snapshot".into());
    }
    let selected = same_version.filter(|current| {
        record::release(root, current).map_or_else(
            || stage::fingerprint(&root.join("releases").join(current)).ok(),
            |release| Some(release.tree),
        ) == Some(known.tree.clone())
    });
    let selected = if let Some(selected) = selected {
        selected
    } else {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let generation = format!(
            "{}+{}.{}-{}",
            descriptor.version,
            &known.tree[..16],
            &nonce[..8],
            target
        );
        fs::create_dir_all(root.join("releases")).map_err(display)?;
        stage::publish(&tree, root, &root.join("releases").join(&generation))?;
        generation
    };
    select_release(root, &selected)?;
    refresh_auto_update_helper(root)?;
    let activated = result_known(root, &held, selected.clone(), &known)?;
    if stage::stat(&bootstrap)? == bootstrap_stat {
        record::write(root, &selected, &bootstrap, bootstrap_stat, &known)?;
    }
    schedule(scheduler, root, &held.data_dir);
    Ok(activated)
}

fn schedule(scheduler: fn(&Path, &Path) -> Result<(), String>, root: &Path, data_dir: &Path) {
    if let Err(error) = scheduler(root, data_dir) {
        eprintln!("Gent automatic-update scheduling was not refreshed: {error}");
    }
}

fn daemon_name() -> &'static str {
    if cfg!(windows) { "gentd.exe" } else { "gentd" }
}

fn result(
    root: &Path,
    held: &lease::ActivationLease,
    selected: String,
    expected: Option<&str>,
) -> Result<ActivationResult, String> {
    let verified = generation_path::open(root, &selected, daemon_name())?;
    let known = record::Known {
        executable: stage::file_digest_handle(&verified.file)?,
        tree: stage::fingerprint(
            verified
                .path
                .parent()
                .ok_or("selected daemon has no runtime directory")?,
        )?,
    };
    if expected.is_some_and(|expected| expected != known.executable) {
        return Err("activated daemon differs from the verified bootstrap".into());
    }
    result_known(root, held, selected, &known)
}

fn result_known(
    root: &Path,
    held: &lease::ActivationLease,
    selected: String,
    known: &record::Known,
) -> Result<ActivationResult, String> {
    if selected_release(root)?.as_deref() != Some(&selected) {
        return Err("runtime selection changed during activation".into());
    }
    let verified = generation_path::open(root, &selected, daemon_name())?;
    Ok(ActivationResult {
        schema: 1,
        daemon_path: verified.path,
        executable_digest_sha256: known.executable.clone(),
        bootstrap_tree_sha256: known.tree.clone(),
        selected_release: selected,
        data_dir: held.data_dir.clone(),
        origin_host_epoch: held.origin_host_epoch,
    })
}

fn verify_bootstrap(root: &Path) -> Result<(), String> {
    for name in required_files() {
        if !fs::symlink_metadata(root.join(name))
            .is_ok_and(|metadata| metadata.is_file() && !files::redirected(&metadata))
        {
            return Err(format!(
                "Gent bootstrap runtime is incomplete: {name} is missing"
            ));
        }
    }
    Ok(())
}

fn expected_target() -> Result<String, String> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Ok("aarch64-apple-darwin".into())
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Ok("x86_64-apple-darwin".into())
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Ok("x86_64-unknown-linux-gnu".into())
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Ok("x86_64-pc-windows-msvc".into())
    } else {
        Err("Gent bootstrap is unsupported on this platform".into())
    }
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
#[path = "runtime_activation_guard_tests.rs"]
mod guard_tests;
#[cfg(test)]
#[path = "runtime_activation_tests.rs"]
mod tests;
#[cfg(all(test, windows))]
#[path = "runtime_activation_windows_tests.rs"]
mod windows_tests;
