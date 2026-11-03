#[cfg(target_os = "linux")]
mod linux {
    //! Explicit owned QA only; exercises the real production cgroup owner, no model/network.
    use gent_linux_cgroup_owner::{CgroupScope, DelegatedParent};
    use gent_types::SandboxResourceLimits;
    use std::{
        io::{Read, Write},
        path::PathBuf,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    pub fn main() {
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
    fn run() -> Result<serde_json::Value, &'static str> {
        let mut input = std::io::stdin();
        let mut len = [0; 4];
        input.read_exact(&mut len).map_err(|_| "config header")?;
        let len = u32::from_be_bytes(len) as usize;
        if !(5..=32768).contains(&len) {
            return Err("config bound");
        }
        let mut data = vec![0; len];
        input.read_exact(&mut data).map_err(|_| "config")?;
        if data[..5] != [1, 0, 0, 0, 0] {
            return Err("config type");
        }
        let config: serde_json::Value =
            serde_json::from_slice(&data[5..]).map_err(|_| "config json")?;
        let nonce = config["nonce"].as_str().ok_or("nonce")?;
        let args = std::env::args().collect::<Vec<_>>();
        let pos = args
            .iter()
            .position(|a| a == "--delegation")
            .ok_or("delegation argument")?;
        let parent = PathBuf::from(args.get(pos + 1).ok_or("delegation value")?);
        let positive = Command::new("/usr/bin/python3")
            .args(["-I", "-S", "-c", "print(sum(range(1000000)))"])
            .output()
            .map_err(|_| "positive control")?;
        if !positive.status.success() {
            return Err("positive control failed");
        }
        let mut scope = CgroupScope::create(
            DelegatedParent::inspect(&parent).map_err(|_| "delegation")?,
            nonce,
            SandboxResourceLimits {
                max_processes: 8,
                max_memory_bytes: 64 * 1024 * 1024,
                max_cpu_time_ms: 50,
            },
        )
        .map_err(|_| "scope")?;
        let scopepath = PathBuf::from("/sys/fs/cgroup").join(
            scope
                .membership_path()
                .map_err(|_| "membership")?
                .trim_start_matches('/'),
        );
        let cpu_max_present = scopepath.join("cpu.max").exists();
        let cpu_stat_present = scopepath.join("cpu.stat").exists();
        if cpu_max_present || !cpu_stat_present {
            return Err("CPU accounting-only prerequisite");
        }
        let mut child=Command::new("/usr/bin/python3").args(["-I","-S","-u","-c","import sys,subprocess;sys.stdin.readline();subprocess.Popen([sys.executable,'-I','-S','-c','while True:pass'])\nwhile True:pass"]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_|"blocked helper")?;
        let stat =
            std::fs::read_to_string(format!("/proc/{}/stat", child.id())).map_err(|_| "birth")?;
        let birth = stat
            .rsplit_once(')')
            .ok_or("birth format")?
            .1
            .split_whitespace()
            .nth(19)
            .ok_or("birth field")?;
        if scope.attach_blocked_helper(&mut child, birth).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("attach");
        }
        child
            .stdin
            .take()
            .ok_or("helper input")?
            .write_all(b"GO\n")
            .map_err(|_| "release")?;
        let until = Instant::now() + Duration::from_secs(3);
        let sample = loop {
            let sample = scope.sample_cpu_budget().map_err(|_| "CPU accounting")?;
            if sample.exceeded {
                break sample;
            }
            if Instant::now() > until {
                return Err("CPU deadline");
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let until = Instant::now() + Duration::from_secs(2);
        let exit = loop {
            if let Some(exit) = child.try_wait().map_err(|_| "wait")? {
                break exit;
            }
            if Instant::now() > until {
                return Err("helper exit deadline");
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        let cleanup = scope
            .terminate_and_remove(Duration::from_secs(2))
            .map_err(|_| "settlement")?;
        Ok(
            serde_json::json!({"schema":1,"passed":!exit.success()&&cleanup.empty&&cleanup.removed,"nonce":nonce,"scopeSettled":cleanup.empty&&cleanup.removed,"positiveControl":true,"cpuMaxPresent":cpu_max_present,"cpuStatPresent":cpu_stat_present,"budgetUsec":sample.budget_usec,"sampleUsec":sample.usage_usec,"hardTotalCeilingClaim":false,"pollMs":5,"cpuControllerDelegated":false}),
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
