#[cfg(target_os = "linux")]
mod linux {
    //! Owned QA: real public Claude version and inherited-pipe descendant timeout.
    //! No auth home, vendor traffic or provider containment evidence is used.
    use std::{
        collections::BTreeMap,
        io::{Read, Write},
        path::{Path, PathBuf},
        time::{Duration, Instant},
    };
    pub fn main() {
        if std::env::args().any(|v| v == "--owner-death-child") {
            let parent = argument("--delegation").unwrap();
            let helper = gent_drivers::lock::capture(
                "gent-linux-helper",
                &argument("--helper").unwrap(),
                "qa",
                "qa",
            )
            .unwrap();
            let shell = gent_drivers::lock::capture(
                "claude",
                Path::new("/usr/bin/dash"),
                "QA system shell",
                "QA owner death fixture",
            )
            .unwrap();
            let env = BTreeMap::from([
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("HOME".into(), "/nonexistent".into()),
            ]);
            let _ = gent_linux_sandbox::host_probe::run(
                &parent,
                &helper,
                &shell,
                Path::new("/tmp"),
                &["-c".into(), "sleep 20 & wait".into()],
                &env,
                Duration::from_secs(30),
            );
            std::process::exit(2);
        }
        let result = run();
        let value = match result {
            Ok(v) => v,
            Err(stage) => serde_json::json!({"passed":false,"stage":stage}),
        };
        let data = serde_json::to_vec(&value).unwrap();
        let mut out = std::io::stdout().lock();
        out.write_all(&u32::try_from(data.len() + 5).unwrap().to_be_bytes())
            .unwrap();
        out.write_all(&[40, 0, 0, 0, 0]).unwrap();
        out.write_all(&data).unwrap();
        out.flush().unwrap();
        std::process::exit(i32::from(value["passed"] != true));
    }
    fn argument(name: &str) -> Result<PathBuf, &'static str> {
        let args: Vec<_> = std::env::args().collect();
        let i = args
            .iter()
            .position(|v| v == name)
            .ok_or("missing argument")?;
        Ok(PathBuf::from(args.get(i + 1).ok_or("missing value")?))
    }
    fn children(parent: &Path) -> Result<Vec<String>, &'static str> {
        let mut names = std::fs::read_dir(parent)
            .map_err(|_| "scope inventory")?
            .filter_map(|e| {
                let e = e.ok()?;
                e.file_type()
                    .ok()?
                    .is_dir()
                    .then(|| e.file_name().to_string_lossy().into_owned())
            })
            .collect::<Vec<_>>();
        names.sort();
        Ok(names)
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
        let mut bytes = vec![0; len];
        std::io::stdin()
            .read_exact(&mut bytes)
            .map_err(|_| "config body")?;
        let parent = argument("--delegation")?;
        let before = children(&parent)?;
        let helper =
            gent_drivers::lock::capture("gent-linux-helper", &argument("--helper")?, "qa", "qa")
                .map_err(|_| "helper identity")?;
        let provider =
            gent_drivers::lock::capture("claude", &argument("--provider")?, "2.1.271", "qa")
                .map_err(|_| "provider identity")?;
        let env = BTreeMap::from([
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), "/nonexistent".into()),
        ]);
        let output = gent_linux_sandbox::host_probe::run(
            &parent,
            &helper,
            &provider,
            Path::new("/tmp"),
            &["--version".into()],
            &env,
            Duration::from_secs(15),
        )?;
        let version =
            output.stderr_empty && String::from_utf8_lossy(&output.stdout).contains("2.1.271");
        let shell = gent_drivers::lock::capture(
            "claude",
            Path::new("/usr/bin/dash"),
            "QA system shell",
            "QA trusted descendant fixture",
        )
        .map_err(|_| "shell identity")?;
        let started = Instant::now();
        let timeout = gent_linux_sandbox::host_probe::run(
            &parent,
            &helper,
            &shell,
            Path::new("/tmp"),
            &["-c".into(), "sleep 20 & wait".into()],
            &env,
            Duration::from_millis(200),
        );
        let elapsed = started.elapsed().as_millis();
        let after = children(&parent)?;
        if before != after {
            return Err("timeout scope leaked");
        }
        let mut owner =
            std::process::Command::new(std::env::current_exe().map_err(|_| "own image")?)
                .arg("--owner-death-child")
                .arg("--delegation")
                .arg(&parent)
                .arg("--helper")
                .arg(argument("--helper")?)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|_| "owner death spawn")?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut scope = None;
        while Instant::now() < deadline {
            for name in children(&parent)? {
                if !before.contains(&name) && name.starts_with("gent-") {
                    let path = parent.join(name);
                    let procs = std::fs::read_to_string(path.join("cgroup.procs"))
                        .map_err(|_| "owner scope")?;
                    if procs.lines().count() >= 3 {
                        scope = Some(path);
                        break;
                    }
                }
            }
            if scope.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let Some(scope) = scope else {
            let _ = owner.kill();
            let _ = owner.wait();
            return Err("owner SDK descendants absent");
        };
        let owner_killed = owner.kill().is_ok();
        let _ = owner.wait();
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(2) {
            if std::fs::read_to_string(scope.join("cgroup.events"))
                .map_err(|_| "owner scope events")?
                .lines()
                .any(|v| v == "populated 0")
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let owner_death_settled = std::fs::read_to_string(scope.join("cgroup.events"))
            .map_err(|_| "owner scope events")?
            .lines()
            .any(|v| v == "populated 0");
        if owner_death_settled {
            std::fs::remove_dir(&scope).map_err(|_| "owner empty scope cleanup")?;
        }
        let owner_death_ms = started.elapsed().as_millis();
        let after = children(&parent)?;
        let passed = version
            && timeout.is_err()
            && elapsed < 5000
            && before == after
            && owner_killed
            && owner_death_settled;
        Ok(
            serde_json::json!({"passed":passed,"versionMatched":version,"descendantTimeoutRejected":timeout.is_err(),"timeoutElapsedMs":elapsed,"scopeChildrenUnchanged":before==after,"ownerSigkillTreeSettled":owner_death_settled,"ownerDeathElapsedMs":owner_death_ms,"noCredentialHome":true,"noModelRequest":true,"notProviderContainmentEvidence":true}),
        )
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
