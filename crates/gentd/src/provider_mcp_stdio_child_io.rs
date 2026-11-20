#[cfg(not(target_os = "linux"))]
use super::super::{CommandWrap, Spec, Stdio};
use super::super::{Mutex, Ticket, Write};

pub(super) fn write_retaining_operation(
    input: &mut impl Write,
    bytes: &[u8],
    operation: Option<Ticket>,
    held: &Mutex<Vec<Ticket>>,
) -> std::io::Result<()> {
    if let Some(ticket) = operation {
        held.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(ticket);
    }
    input.write_all(bytes).and_then(|()| input.flush())
}

#[cfg(not(target_os = "linux"))]
pub(super) fn command(spec: &Spec) -> CommandWrap {
    let mut command = gent_types::hidden_command(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(&spec.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    if let Some(value) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", value);
    }
    let mut command = CommandWrap::from(command);
    #[cfg(windows)]
    {
        command.wrap(process_wrap::std::CreationFlags(
            windows::Win32::System::Threading::CREATE_NO_WINDOW,
        ));
        command.wrap(process_wrap::std::JobObject);
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        command.wrap(process_wrap::std::ProcessGroup::leader());
    }
    command
}
