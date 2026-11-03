use super::{
    LocalStream, Path, PathBuf, connect, daemon_command, default_daemon_binary, unavailable,
    wait_for_spawned_connection,
};
#[cfg(windows)]
use super::{pipe_is_busy, wait_for_connection_until, windows};
pub(crate) async fn connect_or_start(
    data_dir: &Path,
    no_autostart: bool,
) -> Result<LocalStream, Box<dyn std::error::Error>> {
    connect_or_start_with_spawn(data_dir,no_autostart,|| {
        let daemon=std::env::var_os("GENTD_BIN").map_or_else(default_daemon_binary,PathBuf::from);
        let daemon_display=daemon.display().to_string();
        daemon_command(daemon,data_dir).spawn().map_err(|error| unavailable(if error.kind()==std::io::ErrorKind::NotFound {
            format!("gentd was not found at {daemon_display}; build it with `cargo build -p gentd` or set GENTD_BIN to its executable")
        } else {format!("could not start gentd at {daemon_display}: {error}")}).into())
    }).await
}

pub(super) async fn connect_or_start_with_spawn(
    data_dir: &Path,
    no_autostart: bool,
    spawn: impl FnOnce() -> Result<tokio::process::Child, Box<dyn std::error::Error>>,
) -> Result<LocalStream, Box<dyn std::error::Error>> {
    let initial = connect(data_dir).await;
    connect_or_start_after_initial(data_dir, no_autostart, initial, spawn).await
}

pub(super) async fn connect_or_start_after_initial(
    data_dir: &Path,
    no_autostart: bool,
    initial: Result<LocalStream, std::io::Error>,
    spawn: impl FnOnce() -> Result<tokio::process::Child, Box<dyn std::error::Error>>,
) -> Result<LocalStream, Box<dyn std::error::Error>> {
    match initial {
        Ok(stream) => return Ok(stream),
        #[cfg(windows)]
        Err(error) if pipe_is_busy(&error) => {
            return wait_for_connection_until(data_dir, None).await;
        }
        #[cfg(windows)]
        Err(error) if !windows::endpoint_absent(&error) => {
            return Err(unavailable(format!("gentd endpoint connection failed: {error}")).into());
        }
        Err(_) => {}
    }
    if no_autostart {
        return Err(unavailable("gentd is unavailable and --no-autostart was requested").into());
    }
    let mut child = spawn()?;
    wait_for_spawned_connection(data_dir, &mut child).await
}
