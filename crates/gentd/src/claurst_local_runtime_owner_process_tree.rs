use std::process::{Child, Command};

use gent_drivers::{
    interrupt::ProcessTreeSignal,
    process::{configure_process_tree as configure, signal_process_tree},
};

pub(super) fn configure_process_tree(command: &mut Command) {
    configure(command);
}

pub(super) fn shutdown_process_tree(child: &mut Child) -> Result<(), String> {
    let stopped = stop_process_tree(child);
    gent_drivers::process::groups::reaped(child.id());
    stopped
}

fn stop_process_tree(child: &mut Child) -> Result<(), String> {
    let pid = child
        .id()
        .try_into()
        .map_err(|_| "local process id overflowed".to_owned())?;
    if child
        .try_wait()
        .map_err(|error| error.to_string())?
        .is_some()
    {
        let _ = signal_process_tree(pid, ProcessTreeSignal::Kill);
        return Ok(());
    }
    if let Err(error) = signal_process_tree(pid, ProcessTreeSignal::Kill) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.to_string());
    }
    child.wait().map(|_| ()).map_err(|error| error.to_string())
}

#[cfg(all(test, unix))]
#[path = "claurst_local_runtime_owner_process_tree_tests.rs"]
mod tests;
