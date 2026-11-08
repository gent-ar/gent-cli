use std::io::{Read, Write};
use std::time::{Duration, Instant};

use gent_protocol::{Hello, WORKSPACE_GIT_CAPABILITY, WireFrame, WorkspaceGitFrame};
use gent_types::{CapabilitySet, PROTOCOL_MAX, PROTOCOL_MIN};
use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

use crate::transport::tests::{FakeRuntime, SLOW_GIT_STATUS};
use crate::transport_windows::{LISTENING_INSTANCES, serve_named_pipe};

#[tokio::test]
async fn named_pipe_accepts_a_local_client() {
    let name = format!(r"\\.\pipe\gentd-test-{}", std::process::id());
    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)
        .unwrap();
    let client = ClientOptions::new().open(&name).unwrap();
    server.connect().await.unwrap();
    drop(client);
}

#[tokio::test]
async fn a_burst_of_retrying_clients_all_connect_promptly() {
    let name = start_listener("burst").await;
    let attempts = 32;
    let mut connects = tokio::task::JoinSet::new();
    for _ in 0..attempts {
        let name = name.clone();
        connects.spawn_blocking(move || connect_with_retry(&name));
    }

    let mut succeeded = 0;
    while let Some(result) = connects.join_next().await {
        result
            .unwrap()
            .expect("a retrying client must eventually connect");
        succeeded += 1;
    }

    assert_eq!(succeeded, attempts);
}

#[tokio::test]
async fn simultaneous_clients_each_find_a_listening_instance() {
    let name = start_listener("simultaneous").await;

    let clients = (0..LISTENING_INSTANCES)
        .map(|_| open(&name))
        .collect::<Result<Vec<_>, _>>();

    assert!(clients.is_ok(), "{clients:?}");
}

#[test]
fn a_slow_git_status_does_not_delay_another_clients_handshake() {
    let name = format!(r"\\.\pipe\gentd-slow-git-{}", std::process::id());
    let pipe_name = name.clone();
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(serve_named_pipe(&pipe_name, FakeRuntime))
            .map_err(|error| error.to_string())
    });
    std::thread::sleep(Duration::from_millis(300));
    let busy = name.clone();
    let slow = std::thread::spawn(move || {
        let mut pipe = open(&busy)?;
        send(&mut pipe, &hello())?;
        receive(&mut pipe)?;
        send(
            &mut pipe,
            &WorkspaceGitFrame::StatusRequest {
                request_id: "request-1".into(),
                workspace_id: "workspace-1".into(),
            },
        )?;
        receive(&mut pipe)
    });
    std::thread::sleep(Duration::from_millis(300));

    let started = Instant::now();
    let handshake = handshake(&name);
    let elapsed = started.elapsed();

    assert_eq!(handshake.unwrap()["type"], "negotiated");
    assert!(elapsed < SLOW_GIT_STATUS / 3, "handshake took {elapsed:?}");
    assert_eq!(slow.join().unwrap().unwrap()["type"], "status");
}

async fn start_listener(label: &str) -> String {
    let name = format!(r"\\.\pipe\gentd-{label}-{}", std::process::id());
    let pipe_name = name.clone();
    tokio::spawn(async move {
        let _ = serve_named_pipe(&pipe_name, FakeRuntime).await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    name
}

fn open(name: &str) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)
}

fn handshake(name: &str) -> std::io::Result<serde_json::Value> {
    let mut pipe = open(name)?;
    send(&mut pipe, &hello())?;
    receive(&mut pipe)
}

fn hello() -> WireFrame {
    WireFrame::Hello(Hello {
        protocol_min: PROTOCOL_MIN,
        protocol_max: PROTOCOL_MAX,
        capabilities: CapabilitySet(vec![WORKSPACE_GIT_CAPABILITY.into()]),
    })
}

fn send(pipe: &mut std::fs::File, frame: &impl serde::Serialize) -> std::io::Result<()> {
    let body = serde_json::to_vec(frame)?;
    pipe.write_all(&u32::try_from(body.len()).unwrap().to_be_bytes())?;
    pipe.write_all(&body)
}

fn receive(pipe: &mut std::fs::File) -> std::io::Result<serde_json::Value> {
    let mut length = [0_u8; 4];
    pipe.read_exact(&mut length)?;
    let mut body = vec![0_u8; u32::from_be_bytes(length) as usize];
    pipe.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

fn connect_with_retry(name: &str) -> std::io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match open(name) {
            Ok(client) => {
                drop(client);
                return Ok(());
            }
            Err(error) if error.raw_os_error() == Some(231) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error),
        }
    }
}
