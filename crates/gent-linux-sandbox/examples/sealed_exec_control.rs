#[cfg(target_os = "linux")]
mod linux {
    use gent_linux_cgroup_owner::{CgroupScope, DelegatedParent};
    use gent_linux_sandbox::{
        native_runtime::birth,
        sealed_exec::{REQUIRED_SEALS, SealedExecutable},
    };
    use std::{
        collections::BTreeMap,
        io::{Read, Write},
        os::{
            fd::AsRawFd,
            unix::fs::{MetadataExt, PermissionsExt},
        },
        path::PathBuf,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    fn argument(name: &str) -> Result<PathBuf, &'static str> {
        let args: Vec<_> = std::env::args().collect();
        let i = args
            .iter()
            .position(|v| v == name)
            .ok_or("missing argument")?;
        Ok(PathBuf::from(args.get(i + 1).ok_or("missing value")?))
    }
    pub fn main() {
        if std::env::args().any(|v| v == "--sealed-child") {
            let code = child();
            if let Err(stage) = code {
                println!("{}", serde_json::json!({"failureStage":stage}));
                std::process::exit(125);
            }
            return;
        }
        let result = run();
        let value = match result {
            Ok(v) => v,
            Err(stage) => serde_json::json!({"passed":false,"stage":stage}),
        };
        let data = serde_json::to_vec(&value).unwrap();
        let mut out = std::io::stdout().lock();
        out.write_all(&((data.len() + 5) as u32).to_be_bytes())
            .unwrap();
        out.write_all(&[40, 0, 0, 0, 0]).unwrap();
        out.write_all(&data).unwrap();
        out.flush().unwrap();
        std::process::exit(if value["passed"] == true { 0 } else { 1 });
    }
    fn child() -> Result<(), &'static str> {
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|_| "capture gate")?;
        if line != "CAPTURE\n" {
            return Err("capture gate");
        }
        let path = argument("--origin")?;
        std::fs::copy(argument("--source")?, &path).map_err(|_| "user SDK copy")?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "user SDK mode")?;
        let lock = gent_drivers::lock::capture("claude", &path, "2.1.271", "sealed QA")
            .map_err(|_| "origin lock")?;
        let expected = argument("--sha")?.to_string_lossy().into_owned();
        if lock.digest_sha256 != expected {
            return Err("origin publisher pin mismatch");
        }
        let snapshot = SealedExecutable::capture(&lock)?;
        let ready = serde_json::json!({"fd":snapshot.as_fd().as_raw_fd(),"sha256":snapshot.origin().digest_sha256,"bytes":snapshot.bytes(),"pid":std::process::id(),"birth":birth(std::process::id())?,"originUid":std::fs::metadata(&path).map_err(|_|"origin metadata")?.uid(),"originLock":lock});
        println!("{}", ready);
        std::io::stdout().flush().map_err(|_| "ready flush")?;
        line.clear();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|_| "exec gate")?;
        let gate_command = line.clone();
        let env = BTreeMap::from([
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), "/nonexistent".into()),
            ("DISABLE_AUTOUPDATER".into(), "1".into()),
            ("DISABLE_TELEMETRY".into(), "1".into()),
        ]);
        if gate_command == "TRY_BUSY\n" {
            let error = snapshot
                .exec(&["--version".into()], &env)
                .err()
                .ok_or("busy exec unexpectedly returned")?;
            println!(
                "{}",
                serde_json::json!({"busyRejected":error=="sealed exec busy","kernelExecError":error})
            );
            std::io::stdout().flush().map_err(|_| "busy result")?;
            line.clear();
            std::io::stdin()
                .read_line(&mut line)
                .map_err(|_| "final exec gate")?;
        }
        if line != "EXEC\n" {
            return Err("exec gate");
        }
        match snapshot.exec(&["--version".into()], &env) {
            Err(e) => Err(e),
            Ok(v) => match v {},
        }
    }
    fn run() -> Result<serde_json::Value, &'static str> {
        let mut len = [0; 4];
        std::io::stdin()
            .read_exact(&mut len)
            .map_err(|_| "config")?;
        let len = u32::from_be_bytes(len) as usize;
        if !(5..=32768).contains(&len) {
            return Err("config bound");
        }
        let mut data = vec![0; len];
        std::io::stdin()
            .read_exact(&mut data)
            .map_err(|_| "config body")?;
        let config: serde_json::Value =
            serde_json::from_slice(&data[5..]).map_err(|_| "config JSON")?;
        let work = argument("--work-root")?;
        let origin = work.join("user-managed-sdk");
        let source = argument("--provider")?;
        let lock = gent_drivers::lock::capture("claude", &source, "2.1.271", "sealed QA")
            .map_err(|_| "publisher source lock")?;
        let parent =
            DelegatedParent::inspect(&argument("--delegation")?).map_err(|_| "delegation")?;
        let nonce = config["nonce"].as_str().ok_or("nonce")?;
        let mut scope = CgroupScope::create(
            parent,
            nonce,
            gent_types::SandboxResourceLimits {
                max_memory_bytes: 1024 * 1024 * 1024,
                max_processes: 64,
                max_cpu_time_ms: 20_000,
            },
        )
        .map_err(|_| "scope create")?;
        let mut child = Command::new(std::env::current_exe().map_err(|_| "own image")?)
            .arg("--sealed-child")
            .arg("--origin")
            .arg(&origin)
            .arg("--source")
            .arg(&source)
            .arg("--sha")
            .arg(&lock.digest_sha256)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "sealed child")?;
        let generation = birth(child.id())?;
        let result = (|| {
            scope
                .attach_blocked_helper(&mut child, &generation)
                .map_err(|_| "scope attach")?;
            let mut input = child.stdin.take().ok_or("child input")?;
            input
                .write_all(b"CAPTURE\n")
                .map_err(|_| "capture release")?;
            let mut output = child.stdout.take().ok_or("child output")?;
            let flags = rustix::fs::fcntl_getfl(&output).map_err(|_| "pipe flags")?;
            rustix::fs::fcntl_setfl(&output, flags | rustix::fs::OFlags::NONBLOCK)
                .map_err(|_| "pipe flags")?;
            let begun = Instant::now();
            let mut buffer = Vec::new();
            let mut line = None;
            while begun.elapsed() < Duration::from_secs(15) {
                if scope.sample_cpu_budget().map_err(|_| "scope CPU")?.exceeded {
                    return Err("scope CPU exceeded");
                }
                let mut bytes = [0; 4096];
                match output.read(&mut bytes) {
                    Ok(0) => return Err("ready EOF"),
                    Ok(n) => {
                        buffer.extend_from_slice(&bytes[..n]);
                        if buffer.len() > 8192 {
                            return Err("ready bound");
                        }
                        if let Some(i) = buffer.iter().position(|v| *v == b'\n') {
                            line = Some(buffer.drain(..=i).collect::<Vec<_>>());
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => return Err("ready read"),
                }
            }
            let ready: serde_json::Value =
                serde_json::from_slice(&line.ok_or("ready timeout")?).map_err(|_| "ready JSON")?;
            if ready["pid"].as_u64() != Some(u64::from(child.id()))
                || ready["birth"] != generation
                || ready["sha256"] != lock.digest_sha256
                || ready["originUid"].as_u64()
                    != Some(u64::from(rustix::process::geteuid().as_raw()))
            {
                return Err("ready origin binding");
            }
            if ready["failureStage"].is_string() {
                return Ok(
                    serde_json::json!({"passed":false,"failureStage":ready["failureStage"]}),
                );
            }
            let current_origin =
                gent_drivers::lock::capture("claude", &origin, "2.1.271", "sealed QA")
                    .map_err(|_| "origin recheck")?;
            if serde_json::to_value(&current_origin).map_err(|_| "origin JSON")?
                != ready["originLock"]
            {
                return Err("origin generation mismatch");
            }
            let fd = ready["fd"]
                .as_u64()
                .filter(|v| *v < 1024)
                .ok_or("snapshot fd")?;
            let write_probe = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/proc/{}/fd/{fd}", child.id()))
                .map_err(|_| "retained snapshot fd")?;
            let snapshot = std::fs::File::open(format!("/proc/{}/fd/{fd}", child.id()))
                .map_err(|_| "readonly inspection")?;
            let seals = rustix::fs::fcntl_get_seals(&snapshot).map_err(|_| "kernel seals")?;
            if !seals.contains(REQUIRED_SEALS) {
                return Err("kernel seals missing");
            }
            let write_denied =
                rustix::io::pwrite(&write_probe, b"x", 0) == Err(rustix::io::Errno::PERM);
            let grow_denied = write_probe
                .set_len(ready["bytes"].as_u64().ok_or("bytes")? + 1)
                .is_err();
            let metadata = snapshot.metadata().map_err(|_| "snapshot metadata")?;
            // Removing the mutable origin after sealing cannot replace the retained executable.
            std::fs::remove_file(&origin).map_err(|_| "origin delete control")?;
            input
                .write_all(b"TRY_BUSY\n")
                .map_err(|_| "busy exec release")?;
            let mut busy_line = Vec::new();
            let busy_deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < busy_deadline {
                let mut byte = [0];
                match output.read(&mut byte) {
                    Ok(0) => return Err("busy exec EOF"),
                    Ok(_) => {
                        busy_line.push(byte[0]);
                        if byte[0] == b'\n' {
                            break;
                        }
                        if busy_line.len() > 1024 {
                            return Err("busy result bound");
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    Err(_) => return Err("busy result read"),
                }
            }
            let busy: serde_json::Value =
                serde_json::from_slice(&busy_line).map_err(|_| "busy negative JSON")?;
            if busy["busyRejected"] != true {
                return Ok(
                    serde_json::json!({"passed":false,"failureStage":"busy negative rejected unexpectedly","kernelExecError":busy["kernelExecError"]}),
                );
            }
            drop(write_probe);
            input.write_all(b"EXEC\n").map_err(|_| "exec release")?;
            drop(input);
            let mut image_seen = false;
            let mut exit = None;
            while begun.elapsed() < Duration::from_secs(20) {
                if let Ok(m) = std::fs::metadata(format!("/proc/{}/exe", child.id())) {
                    image_seen |= m.dev() == metadata.dev() && m.ino() == metadata.ino();
                }
                let mut bytes = [0; 4096];
                match output.read(&mut bytes) {
                    Ok(n) => {
                        buffer.extend_from_slice(&bytes[..n]);
                        if buffer.len() > 65536 {
                            return Err("SDK output bound");
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => return Err("SDK output"),
                };
                if let Some(status) = child.try_wait().map_err(|_| "SDK exit")? {
                    exit = Some(status.success());
                    break;
                }
                if scope.sample_cpu_budget().map_err(|_| "CPU")?.exceeded {
                    return Err("CPU exceeded");
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            let version = String::from_utf8_lossy(&buffer).trim() == "2.1.271 (Claude Code)";
            let scope_path = PathBuf::from("/sys/fs/cgroup").join(
                scope
                    .membership_path()
                    .map_err(|_| "scope path")?
                    .trim_start_matches('/'),
            );
            let peak = std::fs::read_to_string(scope_path.join("memory.peak"))
                .map_err(|_| "scope memory peak")?
                .trim()
                .parse::<u64>()
                .map_err(|_| "memory peak")?;
            let events = std::fs::read_to_string(scope_path.join("memory.events"))
                .map_err(|_| "scope memory events")?;
            let oom = events
                .lines()
                .find_map(|v| v.strip_prefix("oom_kill "))
                .and_then(|v| v.parse::<u64>().ok())
                .ok_or("OOM counter")?;
            Ok(
                serde_json::json!({"passed":write_denied&&grow_denied&&image_seen&&version&&exit==Some(true),"sourceOwnedByQaUser":true,"originDeletedAfterSeal":true,"kernelSeals":seals.bits(),"kernelWriteDenied":write_denied,"kernelGrowDenied":grow_denied,"kernelExeMatchesRetainedFd":image_seen,"versionMatched":version,"noAuthHome":true,"noModelRequest":true,"notProviderAdmission":true,"writableFdExecBusyRejected":true,"ownedScopeMemoryPeak":peak,"ownedScopeOomKills":oom}),
            )
        })();
        let clean = scope
            .terminate_and_remove(Duration::from_secs(2))
            .map_err(|_| "scope cleanup")?;
        let _ = child.kill();
        let until = Instant::now() + Duration::from_secs(1);
        while child.try_wait().map_err(|_| "child reap")?.is_none() {
            if Instant::now() >= until {
                return Err("child reap unknown");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if !clean.empty || !clean.removed {
            return Err("scope unsettled");
        }
        result
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
