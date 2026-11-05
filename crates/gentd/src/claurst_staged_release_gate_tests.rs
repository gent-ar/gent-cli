use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, Instant};

use gent_ports::ClaurstPermissionReply;
use gent_types::PermissionCategory;
use serde_json::{Value, json};

use crate::claurst_acp_transport::{
    ClaurstAcpPermissionRequest, ClaurstAcpTerminal, ClaurstAcpTransport,
};
use crate::claurst_local_runtime::{ClaurstLocalRuntimePlan, ClaurstLocalRuntimeRequest};
use crate::claurst_standalone_owner::ClaurstStandaloneLauncher;
use crate::local_model_catalog::LocalModelCatalog;

const ECHO_SERVER: &str = r#"import json, sys
for line in sys.stdin:
    m = json.loads(line)
    if "id" not in m:
        continue
    r = {}
    if m["method"] == "initialize":
        r = {"protocolVersion": m["params"].get("protocolVersion"), "capabilities": {"tools": {}}, "serverInfo": {"name": "gate", "version": "1"}}
    elif m["method"] == "tools/list":
        r = {"tools": [{"name": "echo", "description": "Echo", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}}]}
    elif m["method"] == "tools/call":
        open(sys.argv[1], "w").write("called")
        r = {"content": [{"type": "text", "text": "echo"}]}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": m["id"], "result": r}) + "\n")
    sys.stdout.flush()
"#;

fn chunk(delta: Value, finish: Option<&str>) -> String {
    let frame = json!({"id": "gate", "object": "chat.completion.chunk", "created": 0, "model": "gate",
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]});
    format!("data: {frame}\n\n")
}

fn tool_call(id: &str, name: &str, arguments: &Value) -> String {
    let call = json!([{"index": 0, "id": id, "type": "function",
        "function": {"name": name, "arguments": arguments.to_string()}}]);
    chunk(json!({"role": "assistant", "tool_calls": call}), None)
        + &chunk(json!({}), Some("tool_calls"))
}

fn completion(body: &Value) -> String {
    let answered = body["messages"].as_array().map_or(0, |messages| {
        messages.iter().filter(|m| m["role"] == "tool").count()
    });
    let events = match answered {
        0 => tool_call(
            "call_write",
            "Write",
            &json!({"file_path": "gate.txt", "content": "gate"}),
        ),
        1 => tool_call("call_echo", "mcp__gate__echo", &json!({"text": "gate"})),
        _ => {
            chunk(json!({"role": "assistant", "content": "done"}), None)
                + &chunk(json!({}), Some("stop"))
        }
    };
    events + "data: [DONE]\n\n"
}

fn serve_scripted_model() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            let mut length = 0;
            let _ = reader.read_line(&mut request_line);
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                    break;
                }
                if let Some((name, value)) = header.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            let (status, kind, payload) = if request_line.contains("/chat/completions") {
                let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
                ("200 OK", "text/event-stream", completion(&body))
            } else {
                ("404 Not Found", "application/json", "{}".to_owned())
            };
            let mut stream = stream;
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
        }
    });
    port
}

fn next_permission<S: crate::claurst_acp_transport::ClaurstAcpStdio>(
    transport: &mut ClaurstAcpTransport<S>,
    deadline: Instant,
) -> Result<ClaurstAcpPermissionRequest, Option<ClaurstAcpTerminal>> {
    while Instant::now() < deadline {
        let drain = transport.drain(32).map_err(|_| None)?;
        if let Some(permission) = drain.permissions.into_iter().next() {
            return Ok(permission);
        }
        if drain.terminal.is_some() {
            return Err(drain.terminal);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(None)
}

fn terminal<S: crate::claurst_acp_transport::ClaurstAcpStdio>(
    transport: &mut ClaurstAcpTransport<S>,
    deadline: Instant,
) -> Option<ClaurstAcpTerminal> {
    next_permission(transport, deadline).err().flatten()
}

fn gate_roster(root: &Path) -> (Value, Value) {
    let script = root.join("echo.py");
    std::fs::write(&script, ECHO_SERVER).unwrap();
    let python = std::env::var("GENT_GATE_PYTHON").unwrap_or_else(|_| "python".into());
    let args = json!([script, root.join("echo-called")]);
    (
        json!({"name": "gate", "command": python, "args": args, "env": {}}),
        json!({"name": "gate", "command": python, "args": args, "env": []}),
    )
}

#[test]
#[ignore = "release gate: set GENT_STAGED_CLAURST to the staged Claurst executable"]
fn staged_claurst_completes_the_gent_handshake_and_binds_every_permission() {
    let claurst = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(std::env::var("GENT_STAGED_CLAURST").expect("GENT_STAGED_CLAURST"));
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let (settings_server, session_server) = gate_roster(root);
    let catalog = LocalModelCatalog::shipped().unwrap();
    let plan = ClaurstLocalRuntimePlan::build(
        ClaurstLocalRuntimeRequest {
            claurst_executable: std::path::absolute(&claurst).unwrap(),
            llama_server_executable: root.join("llama-server"),
            model_path: root.join("model.gguf"),
            claurst_home: root.join("home"),
            effort: gent_types::AgentChatEffort::Low,
            mode: gent_types::AgentChatMode::Agent,
            permission_mode: gent_types::PermissionMode::AskEveryTime,
            mcp_servers: vec![settings_server],
        },
        catalog.model("qwen3-1-7b-q4-k-m").unwrap(),
        serve_scripted_model(),
    )
    .unwrap();
    std::fs::create_dir_all(plan.settings_path.parent().unwrap()).unwrap();
    std::fs::write(&plan.settings_path, &plan.settings_json).unwrap();
    let stdio = crate::claurst_sandbox_launcher::RoutedClaurstStandaloneLauncher::host()
        .launch_acp(&plan.claurst_acp)
        .unwrap();
    let mut transport = ClaurstAcpTransport::new(stdio);
    let session = transport
        .initialize_session_with_mcp(&workspace, vec![session_server])
        .expect("staged Claurst must answer Gent's initialize and session/new");
    transport.prompt(&session, "write the gate file").unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);

    let write = next_permission(&mut transport, deadline).expect("Write permission");
    assert_eq!(
        (write.tool_use_id.as_str(), write.tool_name.as_str()),
        ("call_write", "Write")
    );
    assert_eq!(write.category, PermissionCategory::Edit);
    assert_eq!(
        write.input,
        Some(json!({"file_path": "gate.txt", "content": "gate"}))
    );
    transport
        .respond_permission(&write.request_id, ClaurstPermissionReply::AllowOnce)
        .unwrap();

    let echo = next_permission(&mut transport, deadline).expect("fixed-roster MCP permission");
    assert_eq!(
        (echo.tool_use_id.as_str(), echo.tool_name.as_str()),
        ("call_echo", "mcp__gate__echo")
    );
    assert_eq!(echo.category, PermissionCategory::Provider);
    assert_eq!(echo.input, Some(json!({"text": "gate"})));
    transport
        .respond_permission(&echo.request_id, ClaurstPermissionReply::Deny)
        .unwrap();

    assert_eq!(
        terminal(&mut transport, deadline),
        Some(ClaurstAcpTerminal::Completed)
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("gate.txt")).unwrap(),
        "gate"
    );
    assert!(!root.join("echo-called").exists());
}
