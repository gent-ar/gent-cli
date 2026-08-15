//! Local transport selection; command composition remains in `main`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use gent_protocol::{Hello, WireFrame, read_frame, write_frame};
use gent_types::{CapabilitySet, PROTOCOL_MAX, PROTOCOL_MIN};
#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

pub(crate) async fn request(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    frame: WireFrame,
) -> Result<WireFrame, Box<dyn std::error::Error>> {
    let data_dir = data_dir.unwrap_or_else(default_data_dir);
    let mut stream = connect_or_start(&data_dir, no_autostart).await?;
    write_frame(
        &mut stream,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![
                "decisions".into(),
                "event-resync".into(),
                "events".into(),
                "host-epoch".into(),
                "receipts".into(),
            ]),
        }),
    )
    .await?;
    match read_frame(&mut stream).await? {
        WireFrame::Negotiated(_) => {}
        WireFrame::Error { message, .. } => return Err(message.into()),
        _ => return Err("daemon did not negotiate protocol".into()),
    }
    write_frame(&mut stream, &frame).await?;
    let response = read_frame(&mut stream).await?;
    if let WireFrame::Error { message, .. } = &response {
        return Err(message.clone().into());
    }
    Ok(response)
}

#[cfg(unix)]
type LocalStream = UnixStream;
#[cfg(windows)]
type LocalStream = NamedPipeClient;

async fn connect_or_start(
    data_dir: &Path,
    no_autostart: bool,
) -> Result<LocalStream, Box<dyn std::error::Error>> {
    match connect(data_dir).await {
        Ok(stream) => return Ok(stream),
        #[cfg(windows)]
        Err(error) if pipe_is_busy(&error) => return wait_for_connection(data_dir).await,
        Err(_) => {}
    }
    if no_autostart {
        return Err("gentd is unavailable and --no-autostart was requested".into());
    }
    std::fs::create_dir_all(data_dir)?;
    let daemon = std::env::var_os("GENTD_BIN").map_or_else(default_daemon_binary, PathBuf::from);
    tokio::process::Command::new(daemon)
        .arg("--data-dir")
        .arg(data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    wait_for_connection(data_dir).await
}

async fn wait_for_connection(data_dir: &Path) -> Result<LocalStream, Box<dyn std::error::Error>> {
    for _ in 0..40 {
        if let Ok(stream) = connect(data_dir).await {
            return Ok(stream);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Err("gentd did not become ready; set GENTD_BIN to the daemon executable".into())
}

#[cfg(unix)]
async fn connect(data_dir: &Path) -> Result<LocalStream, std::io::Error> {
    UnixStream::connect(data_dir.join("gentd.sock")).await
}

#[cfg(windows)]
#[allow(clippy::unused_async)] // Keeps the shared call site transport-agnostic.
async fn connect(data_dir: &Path) -> Result<LocalStream, std::io::Error> {
    ClientOptions::new().open(pipe_name(data_dir))
}

#[cfg(windows)]
fn pipe_is_busy(error: &std::io::Error) -> bool {
    // ERROR_PIPE_BUSY from the Win32 API. Tokio's client docs prescribe retrying it.
    error.raw_os_error() == Some(231)
}

fn default_daemon_binary() -> PathBuf {
    let name = if cfg!(windows) { "gentd.exe" } else { "gentd" };
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(name)))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn default_data_dir() -> PathBuf {
    directories::ProjectDirs::from("ar", "Gent", "Gent").map_or_else(
        || PathBuf::from(".gent"),
        |directories| directories.data_local_dir().to_path_buf(),
    )
}

#[cfg(windows)]
fn pipe_name(data_dir: &Path) -> String {
    format!(r"\\.\pipe\gentd-{:016x}", endpoint_hash(data_dir))
}

#[cfg(windows)]
fn endpoint_hash(data_dir: &Path) -> u64 {
    data_dir
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

#[cfg(all(test, unix))]
mod tests {
    use gent_protocol::{Hello, Negotiated, WireFrame, read_frame, write_frame};
    use gent_types::{CapabilitySet, HostEpoch, HostStatus, PROTOCOL_MAX};
    use tokio::net::UnixListener;

    use super::{default_daemon_binary, default_data_dir, request, wait_for_connection};

    fn status() -> WireFrame {
        WireFrame::Status(HostStatus {
            host_epoch: HostEpoch(1),
            protocol_min: PROTOCOL_MAX,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec!["events".into()]),
        })
    }

    fn server(directory: &tempfile::TempDir, handshake: WireFrame, response: Option<WireFrame>) {
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                WireFrame::Hello(Hello { .. })
            ));
            write_frame(&mut stream, &handshake).await.unwrap();
            if let Some(response) = response {
                assert!(matches!(
                    read_frame(&mut stream).await.unwrap(),
                    WireFrame::StatusRequest
                ));
                write_frame(&mut stream, &response).await.unwrap();
            }
        });
    }

    fn negotiated() -> WireFrame {
        WireFrame::Negotiated(Negotiated {
            protocol: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec!["events".into()]),
        })
    }

    #[tokio::test]
    async fn request_negotiates_then_returns_the_typed_daemon_response() {
        let directory = tempfile::tempdir().unwrap();
        server(&directory, negotiated(), Some(status()));
        assert!(matches!(
            request(
                Some(directory.path().into()),
                true,
                WireFrame::StatusRequest
            )
            .await,
            Ok(WireFrame::Status(_))
        ));
    }

    #[tokio::test]
    async fn request_rejects_handshake_and_command_errors_without_autostarting() {
        let directory = tempfile::tempdir().unwrap();
        server(
            &directory,
            WireFrame::Error {
                code: "upgradeRequired".into(),
                message: "upgrade".into(),
            },
            None,
        );
        assert!(
            request(
                Some(directory.path().into()),
                true,
                WireFrame::StatusRequest
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("upgrade")
        );

        let missing = tempfile::tempdir().unwrap();
        assert!(
            request(Some(missing.path().into()), true, WireFrame::StatusRequest)
                .await
                .unwrap_err()
                .to_string()
                .contains("--no-autostart")
        );
    }

    #[tokio::test]
    async fn request_rejects_unexpected_negotiation_and_command_responses() {
        let directory = tempfile::tempdir().unwrap();
        server(&directory, status(), None);
        assert!(
            request(
                Some(directory.path().into()),
                true,
                WireFrame::StatusRequest
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("did not negotiate")
        );

        let directory = tempfile::tempdir().unwrap();
        server(
            &directory,
            negotiated(),
            Some(WireFrame::Error {
                code: "invalidCommand".into(),
                message: "denied".into(),
            }),
        );
        assert!(
            request(
                Some(directory.path().into()),
                true,
                WireFrame::StatusRequest
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("denied")
        );
    }

    #[test]
    fn defaults_resolve_to_non_empty_local_paths() {
        assert!(default_daemon_binary().file_name().is_some());
        assert!(!default_data_dir().as_os_str().is_empty());
    }

    #[tokio::test]
    async fn wait_for_connection_retries_until_a_listener_is_ready() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("gentd.sock");
        let listener = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let _listener = UnixListener::bind(socket).unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(75)).await;
        });
        assert!(wait_for_connection(directory.path()).await.is_ok());
        listener.await.unwrap();
    }
}
