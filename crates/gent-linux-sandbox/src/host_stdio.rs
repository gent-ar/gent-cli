//! Trusted host MCP process ownership, NOT provider containment or permission evidence.
//! The caller must bind the exact host-approved program lock/configuration and
//! preflight the entire selected set; unsupported profiles must not be filtered.
use crate::host_probe::{self, Owner, Purpose};
use gent_types::RunVersionLock;
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    process::{ChildStderr, ChildStdin, ChildStdout, ExitStatus},
    time::{Duration, Instant},
};

fn absolute(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
}
pub(crate) fn valid_shape(program: &Path, args: &[String], env: &BTreeMap<String, String>) -> bool {
    absolute(program)
        && program.file_name().and_then(|v| v.to_str()) == Some("gent")
        && matches!(args.len(), 5 | 7)
        && args[0] == "--no-autostart"
        && args[1] == "--data-dir"
        && absolute(Path::new(&args[2]))
        && args[3] == "mcp"
        && ["automations", "browser", "canvas", "chat", "forge", "goal"].contains(&args[4].as_str())
        && (args.len() == 5
            || (args[5] == "--conversation-id"
                && !args[6].is_empty()
                && args[6].len() <= 512
                && !args[6].chars().any(char::is_control)))
        && args.iter().all(|v| v.len() <= 16384 && !v.contains('\0'))
        && env.len() <= 2
        && env.iter().all(|(k, v)| match k.as_str() {
            "PATH" => v == "/usr/bin:/bin",
            "HOME" => v.len() <= 4096 && !v.contains('\0') && absolute(Path::new(v)),
            _ => false,
        })
}

/// Finite shape preflight only; executable identity and live authority remain mandatory.
pub fn validate_profile(program: &Path, args: &[String], env: &BTreeMap<String, String>) -> bool {
    valid_shape(program, args, env) || crate::node_entry::valid(program, args, env)
}

pub fn private_owner_directory(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).is_ok_and(|m| {
        m.is_dir()
            && m.uid() == rustix::process::geteuid().as_raw()
            && m.mode().trailing_zeros() >= 6
    })
}

/// Trusted helper pins the data directory independently of the API process.
/// Only its own parent-generation capability may cross the private Start.
pub(crate) fn retain_data_directory(
    args: &[String],
    owner: u32,
) -> Result<(std::fs::File, Vec<String>), &'static str> {
    use std::os::fd::AsRawFd;
    let value = args.get(2).ok_or("MCP data argument")?;
    if value.starts_with("/proc/") {
        let prefix = format!("/proc/{owner}/fd/");
        let number = value
            .strip_prefix(&prefix)
            .ok_or("MCP directory capability owner")?;
        if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
            return Err("MCP directory capability descriptor");
        }
    }
    let file = std::fs::File::open(value).map_err(|_| "MCP retained data directory")?;
    if !file.metadata().map_err(|_| "MCP data metadata")?.is_dir() {
        return Err("MCP data is not directory");
    }
    let mut rewritten = args.to_vec();
    rewritten[2] = format!("/proc/self/fd/{}", file.as_raw_fd());
    Ok((file, rewritten))
}

/// Make only this read-only directory descriptor survive the trusted child's
/// exec. All other descriptors retain their existing CLOEXEC behavior.
/// Only used in the dedicated trusted helper, whose other thread is the
/// cgroup watchdog and never spawns. No unrelated concurrent spawn can inherit
/// this temporary capability. Never use this guard in the multithreaded daemon.
pub(crate) struct DataInheritance<'a> {
    file: &'a std::fs::File,
    flags: rustix::io::FdFlags,
    restored: bool,
}
impl<'a> DataInheritance<'a> {
    pub(crate) fn new(file: &'a std::fs::File) -> Result<Self, &'static str> {
        use std::os::fd::AsRawFd;
        if file.as_raw_fd() < 3 {
            return Err("MCP directory descriptor overlaps stdio");
        }
        let flags = rustix::io::fcntl_getfd(file).map_err(|_| "MCP descriptor flags")?;
        rustix::io::fcntl_setfd(file, flags & !rustix::io::FdFlags::CLOEXEC)
            .map_err(|_| "MCP descriptor inheritance")?;
        Ok(Self {
            file,
            flags,
            restored: false,
        })
    }
    pub(crate) fn restore(mut self) -> Result<(), &'static str> {
        rustix::io::fcntl_setfd(self.file, self.flags).map_err(|_| "MCP descriptor restoration")?;
        self.restored = true;
        Ok(())
    }
}
impl Drop for DataInheritance<'_> {
    fn drop(&mut self) {
        if !self.restored {
            let _ = rustix::io::fcntl_setfd(self.file, self.flags);
        }
    }
}
pub struct Config {
    pub delegation_parent: PathBuf,
    pub helper: RunVersionLock,
    pub program: RunVersionLock,
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub lifetime: Duration,
}
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostMcpConfig")
            .field("arguments", &"[REDACTED]")
            .field("environment", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}
pub struct HostStdioChild {
    owner: Owner,
    status: Option<ExitStatus>,
}
impl std::fmt::Debug for HostStdioChild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostStdioChild")
            .field("pid", &self.owner.child.id())
            .field("settled", &self.status.is_some())
            .finish()
    }
}
impl HostStdioChild {
    pub fn spawn(config: Config) -> Result<Self, &'static str> {
        Self::spawn_bytes(config, &[], None)
    }
    /// Optional initialize-only pipelining. Tool calls must still use the
    /// normal admitted writer/operation ticket path after startup.
    pub fn spawn_pipelined_initialize(
        config: Config,
        initialize: &serde_json::Value,
    ) -> Result<Self, &'static str> {
        if initialize["jsonrpc"] != "2.0"
            || initialize["method"] != "initialize"
            || initialize
                .get("id")
                .is_none_or(|id| !id.is_string() && !id.is_number())
            || !initialize["params"].is_object()
            || initialize.as_object().is_none_or(|m| {
                m.keys()
                    .any(|k| !["jsonrpc", "method", "id", "params"].contains(&k.as_str()))
            })
        {
            return Err("native MCP initial method rejected");
        }
        let mut bytes =
            serde_json::to_vec(initialize).map_err(|_| "native MCP initial encoding")?;
        bytes.push(b'\n');
        Self::spawn_bytes(config, &bytes, None)
    }
    /// The authority is transferred into the exact child owner before Start.
    /// Partial-start Unknown cleanup retains it in that same scope quarantine.
    pub fn spawn_retained(
        config: Config,
        authority: std::sync::Arc<dyn Send + Sync>,
    ) -> Result<Self, &'static str> {
        Self::spawn_bytes(config, &[], Some(authority))
    }
    fn spawn_bytes(
        config: Config,
        initial: &[u8],
        authority: Option<std::sync::Arc<dyn Send + Sync>>,
    ) -> Result<Self, &'static str> {
        if !private_owner_directory(&config.cwd) {
            return Err("native MCP private owner cwd rejected");
        }
        let node = config.program.provider == "node-mcp";
        let valid = if node {
            crate::node_entry::valid(
                Path::new(&config.program.canonical_path),
                &config.args,
                &config.env,
            ) && config.env.get("HOME").map(PathBuf::from).as_ref() == Some(&config.cwd)
        } else {
            valid_shape(
                Path::new(&config.program.canonical_path),
                &config.args,
                &config.env,
            )
        };
        if !valid {
            return Err("native MCP finite Gent profile rejected");
        }
        let (mut owner, input, _) = host_probe::start(host_probe::ProbeSpec {
            parent: &config.delegation_parent,
            helper: &config.helper,
            provider: &config.program,
            cwd: &config.cwd,
            args: &config.args,
            env: &config.env,
            timeout: config.lifetime,
            purpose: if node {
                Purpose::NodeMcpStdio
            } else {
                Purpose::GentMcpStdio
            },
            initial_input: initial,
            retained_authority: authority,
        })?;
        // Startup has already attached the trusted blocked helper. Subsequent
        // bytes belong to the MCP stream; the helper consumes no read-ahead.
        owner.child.stdin = Some(input);
        Ok(Self {
            owner,
            status: None,
        })
    }
    pub fn id(&self) -> u32 {
        self.owner.child.id()
    }
    pub fn stdin(&mut self) -> &mut Option<ChildStdin> {
        &mut self.owner.child.stdin
    }
    pub fn stdout(&mut self) -> &mut Option<ChildStdout> {
        &mut self.owner.child.stdout
    }
    pub fn stderr(&mut self) -> &mut Option<ChildStderr> {
        &mut self.owner.child.stderr
    }
    pub fn terminate(&mut self) -> Result<(), &'static str> {
        if self.status.is_some() {
            return Ok(());
        }
        let inner = self.owner.inner.as_mut().ok_or("native MCP owner absent")?;
        if !host_probe::settle(inner, Instant::now() + Duration::from_secs(2)) {
            return Err("native MCP scope settlement unknown");
        }
        self.status = self
            .owner
            .child
            .try_wait()
            .map_err(|_| "native MCP helper wait")?;
        self.status
            .map(|_| ())
            .ok_or("native MCP helper not reaped")
    }
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, &'static str> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }
        if self
            .owner
            .scope
            .sample_cpu_budget()
            .map_or(true, |v| v.exceeded)
        {
            self.terminate()?;
            return Ok(self.status);
        }
        if self
            .owner
            .child
            .try_wait()
            .map_err(|_| "native MCP child status")?
            .is_some()
        {
            self.terminate()?;
        }
        Ok(self.status)
    }
    pub fn wait(&mut self) -> Result<ExitStatus, &'static str> {
        // Used by owner settlement, never an unbounded std::Child::wait.
        self.terminate()?;
        self.status.ok_or("native MCP scope status absent")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_helper_directory_pin_outlives_api_descriptor() {
        use std::os::fd::AsRawFd;
        let root = tempfile::tempdir().unwrap();
        let original = std::fs::File::open(root.path()).unwrap();
        let mut input = args();
        input[2] = format!("/proc/{}/fd/{}", std::process::id(), original.as_raw_fd());
        let (retained, rewritten) = retain_data_directory(&input, std::process::id()).unwrap();
        drop(original);
        std::fs::write(root.path().join("marker"), b"owned").unwrap();
        assert_eq!(
            std::fs::read(Path::new(&rewritten[2]).join("marker")).unwrap(),
            b"owned"
        );
        let mut command = std::process::Command::new("/usr/bin/test");
        command.args(["-d", &rewritten[2]]);
        let inheritance = DataInheritance::new(&retained).unwrap();
        let status = command.status().unwrap();
        inheritance.restore().unwrap();
        assert!(status.success());
        assert!(
            rustix::io::fcntl_getfd(&retained)
                .unwrap()
                .contains(rustix::io::FdFlags::CLOEXEC)
        );
        let inheritance = DataInheritance::new(&retained).unwrap();
        assert!(
            std::process::Command::new("/definitely-absent-owned-test")
                .spawn()
                .is_err()
        );
        drop(inheritance);
        assert!(
            rustix::io::fcntl_getfd(&retained)
                .unwrap()
                .contains(rustix::io::FdFlags::CLOEXEC)
        );
        drop(retained);
        assert!(std::fs::read(Path::new(&rewritten[2]).join("marker")).is_err());
        input[2] = "/proc/1/fd/0".into();
        assert!(retain_data_directory(&input, std::process::id()).is_err());
    }
    fn args() -> Vec<String> {
        vec![
            "--no-autostart".into(),
            "--data-dir".into(),
            "/owned/profile".into(),
            "mcp".into(),
            "automations".into(),
        ]
    }
    #[test]
    fn finite_native_names_and_arguments_are_case_sensitive() {
        assert!(valid_shape(
            Path::new("/owned/gent"),
            &args(),
            &BTreeMap::new()
        ));
        for bad in [
            "/owned/Gent",
            "/owned/gent.exe",
            "/owned/node",
            "/owned/sh",
            "relative/gent",
        ] {
            assert!(!valid_shape(Path::new(bad), &args(), &BTreeMap::new()));
        }
        let mut bad = args();
        bad[4] = "arbitrary-server".into();
        assert!(!valid_shape(
            Path::new("/owned/gent"),
            &bad,
            &BTreeMap::new()
        ));
        let mut bad = args();
        bad[2] = "/owned/../other".into();
        assert!(!valid_shape(
            Path::new("/owned/gent"),
            &bad,
            &BTreeMap::new()
        ));
    }
    #[test]
    fn loaders_tokens_and_extra_runtime_flags_are_not_accepted() {
        for key in [
            "NODE_OPTIONS",
            "NODE_PATH",
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "PYTHONPATH",
            "ANTHROPIC_AUTH_TOKEN",
            "GENT_MCP_BRIDGE_TOKEN",
        ] {
            assert!(!valid_shape(
                Path::new("/owned/gent"),
                &args(),
                &BTreeMap::from([(key.into(), "canary".into())])
            ));
        }
        let mut extra = args();
        extra.push("--require".into());
        assert!(!valid_shape(
            Path::new("/owned/gent"),
            &extra,
            &BTreeMap::new()
        ));
        assert!(valid_shape(
            Path::new("/owned/gent"),
            &args(),
            &BTreeMap::from([
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("HOME".into(), "/owned/private-home".into())
            ])
        ));
    }
}
