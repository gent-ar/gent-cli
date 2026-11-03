#[cfg(target_os = "linux")]
mod linux {
    use gent_linux_sandbox::host_stdio::{Config, HostStdioChild};
    use std::{
        collections::BTreeMap,
        io::{Read, Write},
        path::{Path, PathBuf},
        time::{Duration, Instant},
    };
    fn arg(name: &str) -> Result<PathBuf, &'static str> {
        let a: Vec<_> = std::env::args().collect();
        let i = a.iter().position(|v| v == name).ok_or("argument absent")?;
        a.get(i + 1)
            .map(PathBuf::from)
            .ok_or("argument value absent")
    }
    fn children(parent: &Path) -> Result<Vec<PathBuf>, &'static str> {
        let mut paths = std::fs::read_dir(parent)
            .map_err(|_| "scope listing")?
            .filter_map(|e| e.ok().map(|v| v.path()))
            .filter(|p| p.is_dir())
            .collect::<Vec<_>>();
        paths.sort();
        Ok(paths)
    }
    fn config() -> Result<Config, &'static str> {
        let root = arg("--work-root")?;
        let data = root.join("native-mcp-profile");
        std::fs::create_dir_all(&data).map_err(|_| "private data")?;
        Ok(Config {
            delegation_parent: arg("--delegation")?,
            helper: gent_drivers::lock::capture(
                "gent-helper",
                &arg("--helper")?,
                "qa",
                "owned-helper",
            )
            .map_err(|_| "helper pin")?,
            program: gent_drivers::lock::capture(
                "gent-mcp",
                &arg("--provider")?,
                "qa-native-cli",
                "owned-native-cli",
            )
            .map_err(|_| "Gent pin")?,
            cwd: root,
            args: vec![
                "--no-autostart".into(),
                "--data-dir".into(),
                data.to_str().ok_or("profile path")?.into(),
                "mcp".into(),
                "automations".into(),
            ],
            env: BTreeMap::from([
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("HOME".into(), "/nonexistent".into()),
            ]),
            lifetime: Duration::from_secs(30),
        })
    }
    fn initialize() -> serde_json::Value {
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"owned-native-qa","version":"1"}}})
    }
    fn output(child: &mut HostStdioChild) -> Result<std::process::ChildStdout, &'static str> {
        let out = child.stdout().take().ok_or("MCP stdout")?;
        let flags = rustix::fs::fcntl_getfl(&out).map_err(|_| "stdout flags")?;
        rustix::fs::fcntl_setfl(&out, flags | rustix::fs::OFlags::NONBLOCK)
            .map_err(|_| "stdout nonblocking")?;
        Ok(out)
    }
    fn rpc(
        child: &mut HostStdioChild,
        out: &mut impl Read,
        id: u64,
        buffer: &mut Vec<u8>,
    ) -> Result<serde_json::Value, &'static str> {
        let until = Instant::now() + Duration::from_secs(8);
        while Instant::now() < until {
            let mut bytes = [0; 4096];
            match out.read(&mut bytes) {
                Ok(n) if n > 0 => buffer.extend_from_slice(&bytes[..n]),
                Ok(0) => return Err("MCP stdout EOF"),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                _ => return Err("MCP stdout read"),
            }
            if buffer.len() > 65536 {
                return Err("MCP response bound");
            }
            while let Some(end) = buffer.iter().position(|b| *b == b'\n') {
                let line: Vec<_> = buffer.drain(..=end).collect();
                let value: serde_json::Value =
                    serde_json::from_slice(&line).map_err(|_| "MCP response JSON")?;
                if value["id"] == id {
                    return Ok(value);
                }
            }
            if child.try_wait()?.is_some() {
                return Err("MCP child exited");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Err("MCP RPC deadline")
    }
    fn owner_child() -> Result<(), &'static str> {
        let mut child = HostStdioChild::spawn_pipelined_initialize(config()?, &initialize())?;
        let mut out = output(&mut child)?;
        let response = rpc(&mut child, &mut out, 1, &mut Vec::new())?;
        if response["result"]["serverInfo"]["name"] != "gent" {
            return Err("Gent initialize identity");
        }
        std::io::stdout()
            .write_all(b"READY\n")
            .map_err(|_| "owner ready")?;
        std::io::stdout().flush().map_err(|_| "owner ready flush")?;
        std::thread::sleep(Duration::from_secs(20));
        drop(child);
        Ok(())
    }
    fn run() -> Result<serde_json::Value, &'static str> {
        use std::os::unix::fs::MetadataExt;
        // Consume only the public bootstrap frame; no SDK/user credentials.
        let mut n = [0; 4];
        std::io::stdin()
            .read_exact(&mut n)
            .map_err(|_| "control input")?;
        let n = u32::from_be_bytes(n) as usize;
        if !(5..=32768).contains(&n) {
            return Err("control input bound");
        }
        let mut data = vec![0; n];
        std::io::stdin()
            .read_exact(&mut data)
            .map_err(|_| "control body")?;
        let parent = arg("--delegation")?;
        let before = children(&parent)?;
        let mut child = HostStdioChild::spawn_pipelined_initialize(config()?, &initialize())?;
        let mut out = output(&mut child)?;
        let mut buffer = Vec::new();
        let init = rpc(&mut child, &mut out, 1, &mut buffer)?;
        if init["result"]["serverInfo"]["name"] != "gent" {
            return Err("Gent initialize response");
        }
        let input = child.stdin().as_mut().ok_or("MCP stdin")?;
        input.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n").and_then(|()|input.flush()).map_err(|_|"MCP catalog request")?;
        let catalog = rpc(&mut child, &mut out, 2, &mut buffer)?;
        let count = catalog["result"]["tools"]
            .as_array()
            .map(Vec::len)
            .ok_or("MCP catalog shape")?;
        if count == 0 {
            return Err("MCP empty catalog");
        }
        child.terminate()?;
        let status = child.try_wait()?.is_some();
        drop(child);
        if !status || children(&parent)? != before {
            return Err("MCP scope settlement");
        }
        let mut owner =
            std::process::Command::new(std::env::current_exe().map_err(|_| "own image")?)
                .args(["--owner-child", "--helper"])
                .arg(arg("--helper")?)
                .arg("--provider")
                .arg(arg("--provider")?)
                .arg("--delegation")
                .arg(&parent)
                .arg("--work-root")
                .arg(arg("--work-root")?)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|_| "owner spawn")?;
        let mut ready = owner.stdout.take().ok_or("owner ready pipe")?;
        let flags = rustix::fs::fcntl_getfl(&ready).map_err(|_| "owner pipe flags")?;
        rustix::fs::fcntl_setfl(&ready, flags | rustix::fs::OFlags::NONBLOCK)
            .map_err(|_| "owner pipe nonblocking")?;
        let mut ready_bytes = Vec::new();
        let until = Instant::now() + Duration::from_secs(8);
        while Instant::now() < until && ready_bytes != b"READY\n" {
            let mut b = [0; 16];
            match ready.read(&mut b) {
                Ok(n) if n > 0 => ready_bytes.extend_from_slice(&b[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                _ => break,
            }
            if ready_bytes.len() > 16 {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let scopes = children(&parent)?
            .into_iter()
            .filter(|p| !before.contains(p))
            .collect::<Vec<_>>();
        if ready_bytes != b"READY\n" || scopes.len() != 1 {
            let _ = owner.kill();
            let _ = owner.wait();
            return Err("owned MCP ready absent");
        }
        let scope = &scopes[0];
        let held = std::fs::metadata(scope).map_err(|_| "owner scope identity")?;
        if !scope.file_name().and_then(|v| v.to_str()).is_some_and(|v| {
            v.len() == 37 && v.starts_with("gent-") && v[5..].bytes().all(|c| c.is_ascii_hexdigit())
        }) || held.uid() != rustix::process::geteuid().as_raw()
        {
            return Err("owner scope invalid");
        }
        owner.kill().map_err(|_| "kill own API owner")?;
        owner.wait().map_err(|_| "reap own API owner")?;
        let begun = Instant::now();
        let mut empty = false;
        while begun.elapsed() < Duration::from_secs(2) {
            empty = std::fs::read_to_string(scope.join("cgroup.events"))
                .map_err(|_| "owner scope events")?
                .lines()
                .any(|l| l == "populated 0");
            if empty {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if !empty {
            return Err("owner death tree live");
        }
        let observed = std::fs::metadata(scope).map_err(|_| "owner scope replaced")?;
        if (held.dev(), held.ino()) != (observed.dev(), observed.ino()) {
            return Err("owner scope changed");
        }
        std::fs::remove_dir(scope).map_err(|_| "remove own empty scope")?;
        let restored = children(&parent)? == before;
        Ok(
            serde_json::json!({"passed":restored,"actualGentInitialize":true,"catalogTools":count,"pipelinedInitializeSameStartWrite":true,"ownerSigkillScopeEmpty":empty,"ownerDeathElapsedMs":begun.elapsed().as_millis(),"scopeChildrenUnchanged":restored,"noModelRequest":true,"noUserProfile":true,"noToolCall":true,"selectedServerCount":1,"fullRosterProved":false,"notProviderContainmentEvidence":true}),
        )
    }
    pub fn main() {
        if std::env::args().any(|v| v == "--owner-child") {
            std::process::exit(i32::from(owner_child().is_err()));
        }
        let v = match run() {
            Ok(v) => v,
            Err(stage) => serde_json::json!({"passed":false,"stage":stage}),
        };
        let bytes = serde_json::to_vec(&v).unwrap();
        let mut out = std::io::stdout().lock();
        out.write_all(&u32::try_from(bytes.len() + 5).unwrap().to_be_bytes())
            .unwrap();
        out.write_all(&[40, 0, 0, 0, 0]).unwrap();
        out.write_all(&bytes).unwrap();
        out.flush().unwrap();
        std::process::exit(i32::from(v["passed"] != true));
    }
}
#[cfg(target_os = "linux")]
fn main() {
    linux::main()
}
#[cfg(not(target_os = "linux"))]
fn main() {
    std::process::exit(78)
}
