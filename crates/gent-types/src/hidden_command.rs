use std::{ffi::OsStr, process::Command};

#[must_use]
pub fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    hidden_command_with_flags(program, 0)
}

#[must_use]
pub fn console_command(program: impl AsRef<OsStr>) -> Command {
    if has_console() {
        Command::new(program)
    } else {
        hidden_command(program)
    }
}

#[cfg(windows)]
#[must_use]
pub fn hidden_command_with_flags(program: impl AsRef<OsStr>, windows_flags: u32) -> Command {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW | windows_flags);
    command
}

#[cfg(not(windows))]
#[must_use]
pub fn hidden_command_with_flags(program: impl AsRef<OsStr>, _windows_flags: u32) -> Command {
    Command::new(program)
}

#[cfg(windows)]
fn has_console() -> bool {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONOUT$")
        .is_ok()
}

#[cfg(not(windows))]
const fn has_console() -> bool {
    true
}

#[cfg(all(test, windows))]
mod tests {
    use super::{console_command, hidden_command};

    #[test]
    fn hidden_and_console_commands_run_console_programs_with_piped_output() {
        for mut command in [hidden_command("cmd"), console_command("cmd")] {
            let output = command.args(["/C", "echo", "hidden"]).output().unwrap();
            assert!(output.status.success());
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hidden");
        }
    }
}
