use gent_types::windows_pipe_name;
use std::path::{Path, PathBuf};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::{ClientOptions, ServerOptions},
};

fn root(unc: bool) -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    if unc {
        format!(r"\\gent-ipc-{id}\share\data")
    } else {
        format!(r"C:\gent-ipc-{id}\data")
    }
}

#[tokio::test]
async fn old_raw_dos_and_unc_owners_accept_new_clients_in_both_separator_directions_without_spawn()
{
    for unc in [false, true] {
        let backslash = root(unc);
        let forward = backslash.replace('\\', "/");
        for (server_root, client_root) in [(&backslash, &forward), (&forward, &backslash)] {
            let name = windows_pipe_name(Path::new(server_root));
            let mut server = ServerOptions::new()
                .first_pipe_instance(true)
                .create(&name)
                .unwrap();
            let client = crate::local_ipc::start::connect_or_start_with_spawn(
                Path::new(client_root),
                false,
                || panic!("an alias owner must be found before any autostart"),
            )
            .await
            .unwrap();
            server.connect().await.unwrap();
            server.write_all(b"hello").await.unwrap();
            let mut client = client;
            let mut bytes = [0; 5];
            client.read_exact(&mut bytes).await.unwrap();
            assert_eq!(&bytes, b"hello");
        }
    }
}

#[tokio::test]
async fn missing_unrelated_and_nonabsence_errors_never_spawn_when_forbidden() {
    let value = root(false);
    let other = value.replace("data", "other");
    let _unrelated = ServerOptions::new()
        .first_pipe_instance(true)
        .create(windows_pipe_name(Path::new(&other)))
        .unwrap();
    let error =
        crate::local_ipc::start::connect_or_start_with_spawn(Path::new(&value), true, || {
            panic!("no-autostart may not spawn")
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("--no-autostart"));
    for code in [5, 87, 109] {
        let error = crate::local_ipc::start::connect_or_start_after_initial(
            Path::new(&value),
            false,
            Err(std::io::Error::from_raw_os_error(code)),
            || panic!("nonabsence must not autostart"),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("endpoint connection failed"));
    }
}

#[tokio::test]
async fn busy_legacy_alias_is_retried_without_falling_through_or_spawning() {
    let backslash = root(false);
    let forward = backslash.replace('\\', "/");
    let name = windows_pipe_name(Path::new(&backslash));
    let first = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)
        .unwrap();
    let occupied = ClientOptions::new().open(&name).unwrap();
    assert_eq!(
        ClientOptions::new().open(&name).unwrap_err().raw_os_error(),
        Some(231)
    );
    let next = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        ServerOptions::new().create(&name).unwrap()
    });
    let client = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        crate::local_ipc::start::connect_or_start_with_spawn(Path::new(&forward), false, || {
            panic!("busy existing owner must never autostart")
        }),
    )
    .await
    .unwrap()
    .unwrap();
    drop((first, occupied, client, next.await.unwrap()));
}

#[tokio::test]
async fn a_connected_protocol_error_is_not_retried_against_another_alias() {
    use gent_protocol::{WireFrame, read_frame, write_frame};
    let backslash = root(false);
    let forward = backslash.replace('\\', "/");
    let mut raw = ServerOptions::new()
        .first_pipe_instance(true)
        .create(windows_pipe_name(Path::new(&forward)))
        .unwrap();
    let alternate = ServerOptions::new()
        .first_pipe_instance(true)
        .create(windows_pipe_name(Path::new(&backslash)))
        .unwrap();
    let owner = tokio::spawn(async move {
        raw.connect().await.unwrap();
        assert!(matches!(
            read_frame(&mut raw).await.unwrap(),
            WireFrame::Hello(_)
        ));
        write_frame(
            &mut raw,
            &WireFrame::Error {
                code: "fixtureProtocolRejected".into(),
                message: "fixture protocol rejection".into(),
            },
        )
        .await
        .unwrap();
    });
    let result = crate::local_ipc::connect_and_negotiate(Some(PathBuf::from(forward)), true).await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("fixture protocol rejection")
    );
    owner.await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), alternate.connect())
            .await
            .is_err()
    );
}

#[test]
fn only_absence_allows_sequential_alias_fallback() {
    let path = Path::new(r"C:/gent-ipc-fixture/data");
    let expected = gent_types::windows_pipe_client_candidates(path);
    let mut seen = Vec::new();
    let chosen = super::open_candidates(path, |candidate| {
        seen.push(candidate.to_owned());
        if seen.len() == expected.len() {
            Ok(candidate.to_owned())
        } else {
            Err(std::io::Error::from_raw_os_error(2))
        }
    })
    .unwrap();
    assert_eq!(seen, expected);
    assert_eq!(chosen, expected.last().unwrap().to_owned());
    for code in [5, 231, 87, 109, 232] {
        let mut seen = Vec::new();
        let error = super::open_candidates::<()>(path, |candidate| {
            seen.push(candidate.to_owned());
            Err(std::io::Error::from_raw_os_error(code))
        })
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(code));
        assert_eq!(seen, vec![expected[0].clone()]);
    }
    let mut attempts = 0;
    let error = super::open_candidates::<()>(path, |_| {
        attempts += 1;
        Err(std::io::Error::from_raw_os_error(3))
    })
    .unwrap_err();
    assert_eq!(attempts, expected.len());
    assert_eq!(error.raw_os_error(), Some(3));
}
