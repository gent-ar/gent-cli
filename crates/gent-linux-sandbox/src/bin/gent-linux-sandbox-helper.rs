#[cfg(target_os = "linux")]
mod linux {
    use gent_linux_sandbox::{
        mxc_baseline,
        native_runtime::{Ready, Start, birth},
    };
    use std::{
        io::{BufRead, Read, Write},
        thread,
        time::Duration,
    };
    fn run() -> Result<i32, &'static str> {
        let mut line = Vec::new();
        let input = std::io::stdin();
        let mut initial = std::io::BufReader::new(input.lock()).take(65537);
        initial
            .read_until(b'\n', &mut line)
            .map_err(|_| "startup input")?;
        if line.len() > 65536 || line.last() != Some(&b'\n') {
            return Err("startup bound");
        }
        drop(initial);
        let start: Start = serde_json::from_slice(&line).map_err(|_| "startup invalid")?;
        if start.schema != 1
            || start.nonce.len() != 32
            || !start
                .nonce
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || start.profile_digest.len() != 64
            || start.wall_ms < 100
            || start.wall_ms > 600_000
        {
            return Err("startup policy");
        }
        if std::fs::read_to_string("/proc/self/cgroup")
            .map_err(|_| "scope unavailable")?
            .trim()
            != format!("0::{}", start.scope)
        {
            return Err("scope mismatch");
        }
        if rustix::process::getppid().map(|pid| pid.as_raw_pid().cast_unsigned())
            != Some(start.parent_pid)
            || birth(start.parent_pid).ok().as_deref() != Some(start.parent_birth.as_str())
        {
            return Err("owner identity");
        }
        if start.executable != start.lock.canonical_path
            || gent_drivers::lock::rechecked_identity(&start.lock).map_err(|_| "provider fence")?
                != start.lock
        {
            return Err("provider changed");
        }
        let required = mxc_baseline::required_existing_roots().map_err(|_| "baseline inspect")?;
        for root in required {
            if !start
                .readonly
                .iter()
                .any(|p| std::fs::canonicalize(p).is_ok_and(|p| root.starts_with(p)))
            {
                return Err("baseline not approved");
            }
            if start.readwrite.iter().any(|p| {
                std::fs::canonicalize(p).is_ok_and(|p| root.starts_with(&p) || p.starts_with(&root))
            }) {
                return Err("system baseline cannot be writable");
            }
        }
        let mut allowed = vec!["PATH", "HOME"];
        if start.gateway_proxy.is_some() {
            allowed.extend([
                "ANTHROPIC_BASE_URL",
                "ANTHROPIC_AUTH_TOKEN",
                "CLAUDE_CONFIG_DIR",
                gent_linux_sandbox::claude_environment::MCP_TOKEN_ENV,
            ]);
            allowed.extend(
                gent_linux_sandbox::claude_environment::FLAGS
                    .iter()
                    .map(|(key, _)| *key),
            );
        }
        if start.environment.len() != allowed.len()
            || allowed.iter().any(|key| {
                start
                    .environment
                    .iter()
                    .filter(|(name, _)| name == key)
                    .count()
                    != 1
            })
            || !start
                .environment
                .iter()
                .any(|(k, v)| k == "PATH" && v == "/usr/bin:/bin")
        {
            return Err("ambient environment rejected");
        }
        if start.gateway_proxy.is_some() {
            for (key, value) in gent_linux_sandbox::claude_environment::FLAGS {
                if !start
                    .environment
                    .iter()
                    .any(|(k, v)| k.as_str() == *key && v.as_str() == *value)
                {
                    return Err("nonessential traffic flags rejected");
                }
            }
            let home = start
                .environment
                .iter()
                .find(|(k, _)| k == "HOME")
                .ok_or("private home")?;
            if !start
                .environment
                .iter()
                .any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == &format!("{}/.claude", home.1))
            {
                return Err("private config directory rejected");
            }
        }
        if start.gateway_proxy.is_some() != start.runtime_root.is_some() {
            return Err("network runtime missing");
        }
        if let Some(runtime) = &start.runtime_root {
            runtime.verify_helper_environment()?;
        }
        // Parent wall watchdog is the primary bound. A later independent SDK timeout
        // remains as defense in depth and makes parent-watchdog tests causal.
        let gate_image = std::env::current_exe().map_err(|_| "contained gate image")?;
        if !start
            .readonly
            .iter()
            .any(|root| gate_image.starts_with(root))
        {
            return Err("contained helper resource not approved");
        }
        let (public_environment, private_environment) =
            gent_linux_sandbox::claude_environment::split_private(
                &start.environment,
                &start.arguments,
            )?;
        let gate_start = gent_linux_sandbox::contained_gate::GateStart {
            schema: 2,
            nonce: start.nonce.clone(),
            lock: start.lock.clone(),
            arguments: start.arguments.clone(),
            private_environment,
            public_environment: public_environment.clone(),
            gateway_proxy: start.gateway_proxy,
        };
        let profile = gent_linux_sandbox::sdk_launch::LaunchProfile {
            executable: gate_image.to_str().ok_or("gate image path")?.into(),
            arguments: vec!["--contained-exec".into()],
            cwd: start.cwd.clone(),
            readwrite: start.readwrite.clone(),
            readonly: start.readonly.clone(),
            denied: vec![],
            environment: public_environment,
            timeout_ms: start.wall_ms.saturating_add(5000).min(600_000),
        };
        let mut process = if let Some(gateway) = start.gateway_proxy {
            let request = gent_linux_sandbox::sdk_launch::request_with_gateway(profile, gateway)
                .map_err(|_| "network request")?;
            mxc_sdk::v1::spawn(
                request,
                mxc_sdk::v1::SpawnOptions {
                    telemetry: Some(mxc_sdk::v1::TelemetryConfig {
                        enabled: Some(false),
                    }),
                },
            )
            .map_err(|_| "network containment unavailable")?
        } else {
            gent_linux_sandbox::sdk_launch::spawn(profile)
                .map_err(|_| "native containment unavailable")?
        };
        let mut worker_input = process.take_stdin().ok_or("worker stdin")?;
        let mut worker_output = process.take_stdout().ok_or("worker stdout")?;
        let mut worker_error = process.take_stderr().ok_or("worker stderr")?;
        serde_json::to_writer(&mut worker_input, &gate_start).map_err(|_| "gate startup write")?;
        worker_input
            .write_all(b"\n")
            .and_then(|()| worker_input.flush())
            .map_err(|_| "gate startup write")?;
        let gate_ready: gent_linux_sandbox::contained_gate::GateReady = serde_json::from_slice(
            &gent_linux_sandbox::contained_gate::line(&mut worker_output, 65536)?,
        )
        .map_err(|_| "gate readiness")?;
        let proof = gent_linux_sandbox::contained_gate::inspect(&start, &gate_ready)?;
        let ready = Ready {
            schema: 2,
            nonce: start.nonce.clone(),
            profile_digest: start.profile_digest.clone(),
            scope: start.scope.clone(),
            pid: std::process::id(),
            birth: birth(std::process::id())?,
            worker_pid: proof.pid,
            worker_birth: proof.birth.clone(),
            worker_namespaces: gate_ready.namespaces,
            sealed_image: gate_ready.sealed_image,
        };
        serde_json::to_writer(std::io::stdout(), &ready).map_err(|_| "ready write")?;
        std::io::stdout()
            .write_all(b"\n")
            .and_then(|()| std::io::stdout().flush())
            .map_err(|_| "ready write")?;
        let mut release = std::fs::File::from(
            rustix::io::dup(std::io::stdin()).map_err(|_| "owner release descriptor")?,
        );
        if gent_linux_sandbox::contained_gate::line(&mut release, 128)?
            != format!("GENT-PROVIDER-GO/1 {}\n", start.nonce).as_bytes()
        {
            return Err("owner release invalid");
        }
        drop(release);
        worker_input
            .write_all(format!("GENT-CONTAINED-GO/1 {}\n", start.nonce).as_bytes())
            .and_then(|()| worker_input.flush())
            .map_err(|_| "gate release")?;
        thread::spawn(move || {
            let _ = std::io::copy(&mut std::io::stdin().lock(), &mut worker_input);
            drop(worker_input);
        });
        let out = thread::spawn(move || {
            let mut output = std::io::stdout().lock();
            let result = std::io::copy(&mut worker_output, &mut output);
            let _ = output.flush();
            result
        });
        let err = thread::spawn(move || {
            let mut output = std::io::stderr().lock();
            let result = std::io::copy(&mut worker_error, &mut output);
            let _ = output.flush();
            result
        });
        let code = loop {
            if rustix::process::getppid().map(|pid| pid.as_raw_pid().cast_unsigned())
                != Some(start.parent_pid)
                || birth(start.parent_pid).ok().as_deref() != Some(start.parent_birth.as_str())
            {
                let _ = process.kill();
                break 125;
            }
            match process.try_wait() {
                Ok(Some(code)) => break code,
                Ok(None) => thread::sleep(Duration::from_millis(2)),
                Err(_) => {
                    let _ = process.kill();
                    break 125;
                }
            }
        };
        drop(process);
        // Output threads belong to this helper process and are bounded by the parent
        // cgroup/wall watchdog. The stdin thread dies with helper process exit.
        let _ = out.join();
        let _ = err.join();
        Ok(code)
    }
    pub(super) fn entry() {
        if std::env::args().nth(1).as_deref() == Some("--trusted-node-mcp") {
            let code = gent_linux_sandbox::host_probe::guest_node_mcp().unwrap_or(125);
            std::process::exit(code);
        }
        if std::env::args().nth(1).as_deref() == Some("--trusted-host-mcp") {
            let code = gent_linux_sandbox::host_probe::guest_mcp().unwrap_or(125);
            std::process::exit(code);
        }
        if std::env::args().nth(1).as_deref() == Some("--host-credential-probe") {
            let code = match gent_linux_sandbox::host_probe::guest() {
                Ok(code) => code,
                Err(reason) => {
                    eprintln!("host credential probe failed: {reason}");
                    125
                }
            };
            std::process::exit(code);
        }
        if std::env::args().nth(1).as_deref() == Some("--contained-exec") {
            if let Err(reason) = gent_linux_sandbox::contained_gate::run_guest() {
                eprintln!("contained exec gate failed: {reason}");
                std::process::exit(125);
            }
            return;
        }
        let code = match run() {
            Ok(code) => code,
            Err(reason) => {
                eprintln!("Linux MXC helper startup failed: {reason}");
                125
            }
        };
        if code < 0 {
            let _ = rustix::process::kill_process(
                rustix::process::getpid(),
                rustix::process::Signal::KILL,
            );
        }
        std::process::exit(code);
    }
}
#[cfg(target_os = "linux")]
fn main() {
    linux::entry();
}
#[cfg(not(target_os = "linux"))]
fn main() {
    std::process::exit(78);
}
