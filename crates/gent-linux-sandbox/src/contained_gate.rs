//! Trusted contained exec gate: provider arguments stay on private stdio and
//! provider execution cannot precede the outer owner's kernel inspection.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateStart {
    pub schema: u8,
    pub nonce: String,
    pub lock: gent_types::RunVersionLock,
    pub arguments: Vec<String>,
    pub gateway_proxy: Option<std::net::SocketAddrV4>,
    pub public_environment: Vec<(String, String)>,
    pub private_environment: Vec<(String, String)>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[derive(Debug)]
pub struct GateReady {
    pub schema: u8,
    pub nonce: String,
    pub namespaces: BTreeMap<String, String>,
    pub sealed_image: crate::sealed_exec::SealedImageReady,
}
pub fn namespaces(pid: u32) -> Result<BTreeMap<String, String>, &'static str> {
    ["user", "net", "pid", "mnt"]
        .into_iter()
        .map(|name| {
            Ok((
                name.into(),
                std::fs::read_link(format!("/proc/{pid}/ns/{name}"))
                    .map_err(|_| "namespace unavailable")?
                    .to_string_lossy()
                    .into_owned(),
            ))
        })
        .collect()
}
pub fn line(input: &mut impl Read, max: u64) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    let mut byte = [0; 1];
    let max = usize::try_from(max).map_err(|_| "gate input bound")?;
    while bytes.len() <= max {
        input.read_exact(&mut byte).map_err(|_| "gate input")?;
        bytes.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    if bytes.len() > max || bytes.last() != Some(&b'\n') {
        return Err("gate input bound");
    }
    Ok(bytes)
}
pub fn run_guest() -> Result<(), &'static str> {
    let mut input = std::fs::File::from(
        rustix::io::dup(std::io::stdin()).map_err(|_| "gate input descriptor")?,
    );
    let start: GateStart =
        serde_json::from_slice(&line(&mut input, 65536)?).map_err(|_| "gate frame")?;
    if start.schema != 2
        || start.nonce.len() != 32
        || !start.nonce.bytes().all(|b| b.is_ascii_hexdigit())
        || gent_drivers::lock::rechecked_identity(&start.lock).map_err(|_| "gate provider image")?
            != start.lock
    {
        return Err("gate identity");
    }
    let (public, _private) =
        crate::claude_environment::split_private(&start.private_environment, &start.arguments)
            .map_err(|_| "gate private environment")?;
    if !public.is_empty() {
        return Err("gate private environment key");
    }
    let status = std::fs::read_to_string("/proc/self/status").map_err(|_| "gate status")?;
    if !status.lines().any(|l| l == "NoNewPrivs:\t1")
        || !status.lines().any(|l| l == "CapEff:\t0000000000000000")
    {
        return Err("gate privileges");
    }
    let snapshot = crate::sealed_exec::SealedExecutable::capture(&start.lock)?;
    let ready = GateReady {
        schema: 2,
        nonce: start.nonce.clone(),
        namespaces: namespaces(std::process::id())?,
        sealed_image: snapshot.ready()?,
    };
    serde_json::to_writer(std::io::stdout(), &ready).map_err(|_| "gate output")?;
    std::io::stdout()
        .write_all(b"\n")
        .and_then(|()| std::io::stdout().flush())
        .map_err(|_| "gate output")?;
    if line(&mut input, 128)? != format!("GENT-CONTAINED-GO/1 {}\n", start.nonce).as_bytes() {
        return Err("gate release rejected");
    }
    drop(input);
    snapshot.source_current()?;
    let (_, leaked) =
        crate::claude_environment::split_private(&start.public_environment, &start.arguments)
            .map_err(|_| "public environment")?;
    if !leaked.is_empty() {
        return Err("private token in public environment");
    }
    let mut environment = BTreeMap::new();
    for (key, value) in start
        .public_environment
        .into_iter()
        .chain(start.private_environment)
    {
        if environment.insert(key, value).is_some() {
            return Err("duplicate exec environment");
        }
    }
    // MXC's pinned Bubblewrap backend adds the proxy after the public profile
    // is submitted. Preserve only its exact finite projection; do not inherit
    // unrelated environment or caller-controlled bypass variables.
    let sdk_environment = SDK_PROXY_KEYS
        .iter()
        .copied()
        .chain(["NO_PROXY", "no_proxy"])
        .filter_map(|key| std::env::var(key).ok().map(|value| (key.to_owned(), value)))
        .collect::<BTreeMap<_, _>>();
    add_sdk_proxy_environment(&mut environment, start.gateway_proxy, &sdk_environment)?;
    match snapshot.exec(&start.arguments, &environment) {
        Err(reason) => Err(reason),
        Ok(value) => match value {},
    }
}
/// Retained namespace handles and directory handle prevent proof from being
/// represented merely as a recycled PID. They remain owned by the outer helper.
#[derive(Debug)]
pub struct KernelGateProof {
    pub pid: u32,
    pub birth: String,
    owned_process: std::fs::File,
    owned_pidfd: std::fs::File,
    uid: u32,
    gid: u32,
    scope: String,
    owned_namespaces: Vec<std::fs::File>,
    image: crate::sealed_exec::RetainedImage,
    helper_device: u64,
    helper_inode: u64,
    executed: std::sync::atomic::AtomicBool,
}
fn mount_path(value: &str) -> Result<std::path::PathBuf, &'static str> {
    let bytes = value.as_bytes();
    let mut result = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\' {
            if at + 3 >= bytes.len()
                || !bytes[at + 1..at + 4]
                    .iter()
                    .all(|b| (b'0'..=b'7').contains(b))
            {
                return Err("mount path escape");
            }
            let n = u16::from(bytes[at + 1] - b'0') * 64
                + u16::from(bytes[at + 2] - b'0') * 8
                + u16::from(bytes[at + 3] - b'0');
            if n == 0 || n > 255 {
                return Err("mount path NUL");
            }
            result.push(u8::try_from(n).map_err(|_| "mount path NUL")?);
            at += 4;
        } else {
            result.push(bytes[at]);
            at += 1;
        }
    }
    Ok(std::path::PathBuf::from(
        String::from_utf8(result).map_err(|_| "mount path encoding")?,
    ))
}
fn verify_mounts(pid: u32, start: &crate::native_runtime::Start) -> Result<(), &'static str> {
    use std::os::unix::fs::MetadataExt;
    let text = std::fs::read_to_string(format!("/proc/{pid}/mountinfo"))
        .map_err(|_| "gate mount inspection")?;
    if text.len() > 1024 * 1024 {
        return Err("mount table bound");
    }
    let mut mounts = Vec::new();
    for line in text.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 10 || !fields.contains(&"-") {
            return Err("mount table invalid");
        }
        mounts.push((
            mount_path(fields[4])?,
            fields[5].split(',').any(|flag| flag == "ro"),
        ));
    }
    let guest_root = std::path::PathBuf::from(format!("/proc/{pid}/root"));
    let host = std::fs::metadata("/").map_err(|_| "host root")?;
    let guest = std::fs::metadata(&guest_root).map_err(|_| "guest root")?;
    if (host.dev(), host.ino()) == (guest.dev(), guest.ino()) {
        return Err("host root exposed");
    }
    for (roots, expected_ro) in [(&start.readonly, true), (&start.readwrite, false)] {
        for root in roots {
            let source = std::path::Path::new(root);
            let source_meta = std::fs::metadata(source).map_err(|_| "resource source")?;
            let guest_meta = std::fs::metadata(
                guest_root.join(source.strip_prefix("/").map_err(|_| "resource path")?),
            )
            .map_err(|_| "resource not mounted")?;
            if (source_meta.dev(), source_meta.ino()) != (guest_meta.dev(), guest_meta.ino()) {
                return Err("resource identity changed");
            }
            let closest = mounts
                .iter()
                .filter(|(path, _)| source.starts_with(path))
                .max_by_key(|(path, _)| path.components().count())
                .ok_or("resource mount missing")?;
            if closest.1 != expected_ro {
                return Err("resource mount access mismatch");
            }
        }
    }
    Ok(())
}
pub fn inspect(
    start: &crate::native_runtime::Start,
    ready: &GateReady,
) -> Result<KernelGateProof, &'static str> {
    use std::os::unix::fs::MetadataExt;
    if ready.schema != 2 || ready.nonce != start.nonce || ready.namespaces.len() != 4 {
        return Err("gate readiness binding");
    }
    let outer = namespaces(std::process::id())?;
    if ready
        .namespaces
        .iter()
        .any(|(k, v)| outer.get(k) == Some(v) || !outer.contains_key(k))
    {
        return Err("gate namespace not isolated");
    }
    let pids = std::fs::read_to_string(format!("/sys/fs/cgroup{}/cgroup.procs", start.scope))
        .map_err(|_| "gate scope inspection")?;
    if pids.len() > 65536 {
        return Err("scope task bound");
    }
    let mut matched = None;
    for word in pids.split_whitespace() {
        let pid = word.parse::<u32>().map_err(|_| "scope PID")?;
        if namespaces(pid).ok().as_ref() != Some(&ready.namespaces) {
            continue;
        }
        let birth = crate::native_runtime::birth(pid)?;
        let path = std::path::PathBuf::from(format!("/proc/{pid}/exe"));
        let observed = gent_drivers::lock::capture(
            &start.helper_lock.provider,
            &path,
            &start.helper_lock.version,
            &start.helper_lock.compatibility_entry,
        )
        .map_err(|_| "gate executable")?;
        if observed != start.helper_lock {
            continue;
        }
        if matched.is_some() {
            return Err("ambiguous contained gate");
        }
        let status = std::fs::read_to_string(format!("/proc/{pid}/status"))
            .map_err(|_| "gate process status")?;
        for expected in [
            "NoNewPrivs:\t1",
            "CapEff:\t0000000000000000",
            "CapPrm:\t0000000000000000",
            "CapAmb:\t0000000000000000",
        ] {
            if !status.lines().any(|l| l == expected) {
                return Err("gate process privilege");
            }
        }
        if std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
            .map_err(|_| "gate cgroup")?
            .trim()
            != format!("0::{}", start.scope)
        {
            return Err("gate cgroup changed");
        }
        let host_uid = rustix::process::geteuid().as_raw();
        let host_group = rustix::process::getegid().as_raw();
        for (name, expected) in [("uid_map", host_uid), ("gid_map", host_group)] {
            let map = std::fs::read_to_string(format!("/proc/{pid}/{name}"))
                .map_err(|_| "gate identity map")?;
            let fields = map.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 3
                || fields[0] != expected.to_string()
                || fields[1] != expected.to_string()
                || fields[2] != "1"
            {
                return Err("gate identity mapping");
            }
        }
        verify_mounts(pid, start)?;
        let process =
            std::fs::File::open(format!("/proc/{pid}")).map_err(|_| "gate process handle")?;
        let mut handles = Vec::new();
        for name in ["user", "net", "pid", "mnt"] {
            handles.push(
                std::fs::File::open(format!("/proc/{pid}/ns/{name}"))
                    .map_err(|_| "gate namespace handle")?,
            );
        }
        if crate::native_runtime::birth(pid)? != birth || namespaces(pid)? != ready.namespaces {
            return Err("gate generation changed");
        }
        let raw = i32::try_from(pid).map_err(|_| "gate pid range")?;
        let kernel_pid = rustix::process::Pid::from_raw(raw).ok_or("gate pid")?;
        let pidfd = std::fs::File::from(
            rustix::process::pidfd_open(kernel_pid, rustix::process::PidfdFlags::empty())
                .map_err(|_| "gate pidfd unavailable")?,
        );
        let image =
            crate::sealed_exec::RetainedImage::inspect(pid, &ready.sealed_image, &start.lock)?;
        let helper_meta = std::fs::metadata(&start.helper_lock.canonical_path)
            .map_err(|_| "helper image metadata")?;
        matched = Some(KernelGateProof {
            pid,
            birth,
            owned_process: process,
            owned_pidfd: pidfd,
            uid: host_uid,
            gid: host_group,
            scope: start.scope.clone(),
            owned_namespaces: handles,
            image,
            helper_device: helper_meta.dev(),
            helper_inode: helper_meta.ino(),
            executed: std::sync::atomic::AtomicBool::new(false),
        });
    }
    matched.ok_or("contained gate not found")
}

impl std::fmt::Debug for GateStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GateStart")
            .field("schema", &self.schema)
            .field("arguments", &self.arguments.len())
            .field("private_environment", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

fn pidfd_alive(fd: &std::fs::File) -> bool {
    let mut descriptors = [rustix::event::PollFd::new(fd, rustix::event::PollFlags::IN)];
    matches!(
        rustix::event::poll(
            &mut descriptors,
            Some(&rustix::event::Timespec {
                tv_sec: 0,
                tv_nsec: 0
            })
        ),
        Ok(0)
    ) && descriptors[0].revents().is_empty()
}
impl KernelGateProof {
    pub(crate) fn exited(&self) -> bool {
        !pidfd_alive(&self.owned_pidfd)
    }
    fn proc_text(&self, name: &str) -> Result<String, &'static str> {
        let mut file = std::fs::File::from(
            rustix::fs::openat(
                &self.owned_process,
                name,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::CLOEXEC
                    | rustix::fs::OFlags::NOFOLLOW,
                rustix::fs::Mode::empty(),
            )
            .map_err(|_| "retained process unavailable")?,
        );
        let mut text = String::new();
        std::io::Read::take(&mut file, 65537)
            .read_to_string(&mut text)
            .map_err(|_| "retained process read")?;
        if text.len() > 65536 {
            return Err("retained process bound");
        }
        Ok(text)
    }
    pub(crate) fn executed(&self) -> bool {
        self.executed.load(std::sync::atomic::Ordering::Acquire)
    }
    pub(crate) fn image_proof(&self) -> Result<serde_json::Value, &'static str> {
        if !self.executed() {
            return Err("provider has not executed sealed image");
        }
        self.image.public_proof()
    }
    pub(crate) fn current_status(&self) -> Result<String, &'static str> {
        use std::os::unix::fs::MetadataExt;
        if !pidfd_alive(&self.owned_pidfd) {
            return Err("worker exited");
        }
        if self.image.matches_executable(self.pid)? {
            self.executed
                .store(true, std::sync::atomic::Ordering::Release);
        } else {
            let current = std::fs::metadata(format!("/proc/{}/exe", self.pid))
                .map_err(|_| "worker executable unavailable")?;
            if self.executed()
                || current.dev() != self.helper_device
                || current.ino() != self.helper_inode
            {
                return Err("worker executed an unapproved image");
            }
        }
        let stat = self.proc_text("stat")?;
        if stat
            .rsplit_once(')')
            .and_then(|(_, tail)| tail.split_whitespace().nth(19))
            != Some(self.birth.as_str())
        {
            return Err("worker generation changed");
        }
        let status = self.proc_text("status")?;
        let unique = |key: &str| -> Result<&str, &'static str> {
            let mut values = status.lines().filter_map(|line| line.strip_prefix(key));
            let value = values.next().ok_or("worker status field")?;
            if values.next().is_some() {
                return Err("worker status ambiguity");
            }
            Ok(value)
        };
        for key in ["CapEff:\t", "CapPrm:\t", "CapAmb:\t"] {
            if unique(key)? != "0000000000000000" {
                return Err("worker capabilities changed");
            }
        }
        if unique("NoNewPrivs:\t")? != "1" {
            return Err("worker privilege fence changed");
        }
        for (key, expected) in [("Uid:\t", self.uid), ("Gid:\t", self.gid)] {
            let values = unique(key)?.split_whitespace().collect::<Vec<_>>();
            if values.len() != 4
                || values
                    .iter()
                    .any(|value| value.parse::<u32>().ok() != Some(expected))
            {
                return Err("worker identity changed");
            }
        }
        if self.proc_text("cgroup")?.trim() != format!("0::{}", self.scope) {
            return Err("worker cgroup changed");
        }
        for (name, held) in ["user", "net", "pid", "mnt"]
            .iter()
            .zip(&self.owned_namespaces)
        {
            let now = std::fs::metadata(format!("/proc/{}/ns/{name}", self.pid))
                .map_err(|_| "worker namespace unavailable")?;
            let original = held
                .metadata()
                .map_err(|_| "retained namespace unavailable")?;
            if now.dev() != original.dev() || now.ino() != original.ino() {
                return Err("worker namespace changed");
            }
        }
        if !pidfd_alive(&self.owned_pidfd) {
            return Err("worker exited during proof");
        }
        Ok(status)
    }
}
#[cfg(test)]
mod live_pidfd_tests {
    #[test]
    fn retained_pidfd_reports_exit_without_rebinding_a_reused_pid() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("10")
            .spawn()
            .unwrap();
        let pid = rustix::process::Pid::from_raw(i32::try_from(child.id()).unwrap()).unwrap();
        let fd = std::fs::File::from(
            rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()).unwrap(),
        );
        assert!(super::pidfd_alive(&fd));
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!super::pidfd_alive(&fd));
    }
}

const SDK_PROXY_KEYS: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "FTP_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "ftp_proxy",
];
fn add_sdk_proxy_environment(
    environment: &mut BTreeMap<String, String>,
    gateway: Option<std::net::SocketAddrV4>,
    sdk: &BTreeMap<String, String>,
) -> Result<(), &'static str> {
    if environment
        .keys()
        .any(|k| SDK_PROXY_KEYS.contains(&k.as_str()) || k.eq_ignore_ascii_case("no_proxy"))
    {
        return Err("caller proxy environment rejected");
    }
    if let Some(gateway) = gateway {
        if !gateway.ip().is_loopback() || gateway.port() == 0 {
            return Err("proxy binding rejected");
        }
        // Pinned MXC translates host loopback to slirp host gateway and retains
        // the exact port. All eight values must attest that same translation.
        let expected = format!("http://10.0.2.2:{}/", gateway.port());
        if SDK_PROXY_KEYS
            .iter()
            .any(|k| sdk.get(*k) != Some(&expected))
            || sdk.contains_key("NO_PROXY")
            || sdk.contains_key("no_proxy")
        {
            return Err("SDK proxy projection rejected");
        }
        for key in SDK_PROXY_KEYS {
            environment.insert((*key).into(), expected.clone());
        }
    }
    Ok(())
}
#[cfg(test)]
mod sealed_proxy_tests {
    use super::*;
    #[test]
    fn preserves_only_exact_sdk_route_and_never_ambient_secrets() {
        let gateway = "127.0.0.1:32123".parse().unwrap();
        let mut sdk = SDK_PROXY_KEYS
            .iter()
            .map(|k| ((*k).into(), "http://10.0.2.2:32123/".into()))
            .collect::<BTreeMap<String, String>>();
        sdk.insert("UNREVIEWED_SECRET".into(), "canary-never-copied".into());
        let mut env = BTreeMap::from([("HOME".into(), "/owned/home".into())]);
        add_sdk_proxy_environment(&mut env, Some(gateway), &sdk).unwrap();
        assert_eq!(env.len(), 9);
        assert!(!env.contains_key("UNREVIEWED_SECRET"));
        for key in SDK_PROXY_KEYS {
            assert_eq!(env.get(*key).unwrap(), "http://10.0.2.2:32123/");
        }
    }
    #[test]
    fn mismatched_missing_bypassed_or_caller_proxy_is_rejected() {
        let gateway = "127.0.0.1:32123".parse().unwrap();
        let good = SDK_PROXY_KEYS
            .iter()
            .map(|k| ((*k).into(), "http://10.0.2.2:32123/".into()))
            .collect::<BTreeMap<String, String>>();
        for key in SDK_PROXY_KEYS {
            let mut sdk = good.clone();
            sdk.remove(*key);
            assert!(add_sdk_proxy_environment(&mut BTreeMap::new(), Some(gateway), &sdk).is_err());
            sdk.insert((*key).into(), "http://10.0.2.2:32124".into());
            assert!(add_sdk_proxy_environment(&mut BTreeMap::new(), Some(gateway), &sdk).is_err());
        }
        let mut bypass = good.clone();
        bypass.insert("no_proxy".into(), "*".into());
        assert!(add_sdk_proxy_environment(&mut BTreeMap::new(), Some(gateway), &bypass).is_err());
        assert!(
            add_sdk_proxy_environment(
                &mut BTreeMap::from([("HTTP_PROXY".into(), "http://attacker".into())]),
                Some(gateway),
                &good
            )
            .is_err()
        );
        assert!(
            add_sdk_proxy_environment(
                &mut BTreeMap::new(),
                Some("1.2.3.4:32123".parse().unwrap()),
                &good
            )
            .is_err()
        );
        let mut env = BTreeMap::new();
        add_sdk_proxy_environment(&mut env, None, &good).unwrap();
        assert!(env.is_empty());
    }
}
