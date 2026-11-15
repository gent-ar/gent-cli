//! Signed Windows dispatcher for immutable Gent release pairs.
//!
//! The installer places identical copies at `bin/gent.exe` and `bin/gentd.exe`.
//! It deliberately uses `Command` with `OsString` arguments, never a shell.

#[cfg(windows)]
mod windows_release_selection;

#[cfg(not(windows))]
fn main() {
    eprintln!("gent-launcher is only used by the Windows installer");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    if let Err(error) = launch() {
        eprintln!("Gent launcher failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn launch() -> Result<(), String> {
    use std::ffi::OsString;
    use std::path::Path;

    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let binary = executable
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or("launcher executable name is invalid")?;
    if binary != "gent" && binary != "gentd" {
        return Err("launcher must be named gent.exe or gentd.exe".into());
    }
    let root = executable
        .parent()
        .and_then(Path::parent)
        .ok_or("launcher installation root is invalid")?;
    let target = windows_release_selection::selected_binary(root, binary)?;
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    let status = gent_types::console_command(target)
        .args(arguments)
        .status()
        .map_err(|error| format!("could not start selected release: {error}"))?;
    std::process::exit(status.code().unwrap_or(1));
}
