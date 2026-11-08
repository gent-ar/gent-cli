mod support;

use std::{path::Path, time::Duration};

use gent_protocol::{
    Hello, WireFrame,
    model_catalog::{LOCAL_MODEL_FILES_CAPABILITY, MODEL_CATALOG_CAPABILITY},
    read_frame, read_json_frame, write_frame, write_json_frame,
};
use gent_types::{CapabilitySet, PROTOCOL_MAX, PROTOCOL_MIN};
use serde_json::{Value, json};
use support::{Gentd, gentd};
use tokio::io::{AsyncRead, AsyncWrite};

fn tiny_gguf(name: &str) -> Vec<u8> {
    let mut entries = Vec::new();
    let string = |bytes: &mut Vec<u8>, value: &str| {
        bytes.extend((value.len() as u64).to_le_bytes());
        bytes.extend(value.as_bytes());
    };
    for (key, value) in [
        ("general.architecture", "llama"),
        ("general.name", name),
        ("tokenizer.chat_template", "{{ messages }}"),
    ] {
        string(&mut entries, key);
        entries.extend(8_u32.to_le_bytes());
        string(&mut entries, value);
    }
    string(&mut entries, "llama.context_length");
    entries.extend(4_u32.to_le_bytes());
    entries.extend(8_192_u32.to_le_bytes());
    let mut bytes = b"GGUF".to_vec();
    bytes.extend(3_u32.to_le_bytes());
    bytes.extend(0_u64.to_le_bytes());
    bytes.extend(4_u64.to_le_bytes());
    bytes.extend(entries);
    bytes
}

#[cfg(unix)]
fn open(data_dir: &Path) -> std::io::Result<impl AsyncRead + AsyncWrite + Unpin> {
    let stream = std::os::unix::net::UnixStream::connect(gent_types::local_socket_path(data_dir))?;
    stream.set_nonblocking(true)?;
    tokio::net::UnixStream::from_std(stream)
}

#[cfg(windows)]
fn open(data_dir: &Path) -> std::io::Result<impl AsyncRead + AsyncWrite + Unpin> {
    tokio::net::windows::named_pipe::ClientOptions::new()
        .open(gent_types::windows_pipe_name(data_dir))
}

async fn connect(data_dir: &Path, daemon: &mut Gentd) -> impl AsyncRead + AsyncWrite + Unpin {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut stream = loop {
        if let Ok(stream) = open(data_dir) {
            break stream;
        }
        assert!(daemon.0.try_wait().unwrap().is_none(), "gentd exited early");
        assert!(
            tokio::time::Instant::now() < deadline,
            "gentd never listened"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    write_frame(
        &mut stream,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![
                MODEL_CATALOG_CAPABILITY.into(),
                LOCAL_MODEL_FILES_CAPABILITY.into(),
            ]),
        }),
    )
    .await
    .unwrap();
    let WireFrame::Negotiated(answer) = read_frame(&mut stream).await.unwrap() else {
        panic!("expected negotiation");
    };
    assert!(
        answer
            .capabilities
            .0
            .contains(&LOCAL_MODEL_FILES_CAPABILITY.to_owned())
    );
    stream
}

async fn request(stream: &mut (impl AsyncRead + AsyncWrite + Unpin), frame: Value) -> Value {
    write_json_frame(stream, &frame).await.unwrap();
    read_json_frame(stream).await.unwrap()
}

fn user_model(reply: &Value) -> Option<Value> {
    reply["body"]["catalog"]["providers"]
        .as_array()?
        .iter()
        .filter(|provider| provider["provider"] == "claurst")
        .flat_map(|provider| provider["models"].as_array().cloned().unwrap_or_default())
        .find(|model| model["local"]["removable"] == true)
}

#[tokio::test]
async fn a_picked_gguf_is_registered_listed_selected_and_removed_over_real_ipc() {
    let data = tempfile::tempdir().unwrap();
    let picked = tempfile::tempdir().unwrap();
    let model = picked.path().join("Tiny.gguf");
    std::fs::write(&model, tiny_gguf("Tiny Llama")).unwrap();
    let mut daemon = Gentd(
        gentd(data.path())
            .arg("--standalone-authority")
            .spawn()
            .unwrap(),
    );
    let mut stream = connect(data.path(), &mut daemon).await;

    let added = request(
        &mut stream,
        json!({"type":"addLocalModel","body":{"requestId":"add","path":model.to_str().unwrap()}}),
    )
    .await;
    assert_eq!(added["type"], "modelCatalog", "{added}");
    let listed = user_model(&added).expect("the picked model is listed");
    let id = listed["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("user-tiny-llama-"));
    assert_eq!(listed["label"], "Tiny Llama");
    assert_eq!(listed["local"]["fileName"], "Tiny.gguf");
    assert_eq!(listed["local"]["install"]["state"], "ready");
    assert!(!added.to_string().contains(picked.path().to_str().unwrap()));
    assert_eq!(std::fs::read_dir(picked.path()).unwrap().count(), 1);
    assert!(!data.path().join("models").join(&id).exists());

    let selected = request(
        &mut stream,
        json!({"type":"setDefaultModel","body":{"requestId":"default","selection":{"provider":"claurst","model":id}}}),
    )
    .await;
    assert_eq!(selected["body"]["catalog"]["defaultSelection"]["model"], id);

    let rejected = request(
        &mut stream,
        json!({"type":"addLocalModel","body":{"requestId":"bad","path":picked.path().join("absent.gguf").to_str().unwrap()}}),
    )
    .await;
    assert_eq!(rejected["type"], "error");
    assert_eq!(rejected["body"]["code"], "localModelUnreadable");

    std::fs::remove_file(&model).unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let read = request(
            &mut stream,
            json!({"type":"readModelCatalog","body":{"requestId":"read","refresh":false}}),
        )
        .await;
        if user_model(&read).is_some_and(|model| model["local"]["install"]["state"] == "missing") {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "{read}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let removed = request(
        &mut stream,
        json!({"type":"removeLocalModel","body":{"requestId":"remove","modelId":id}}),
    )
    .await;
    assert_eq!(removed["type"], "modelCatalog", "{removed}");
    assert!(user_model(&removed).is_none());
    assert_eq!(
        removed["body"]["catalog"]["defaultSelection"]["model"],
        gent_protocol::DEFAULT_LOCAL_MODEL_ID
    );
}
