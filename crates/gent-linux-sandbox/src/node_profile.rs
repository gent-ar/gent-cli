//! Finite trusted-host Node MCP contract. Not a hostile JavaScript sandbox.
//! Identity locks, sealed script authority, cgroup owner and live permission
//! evidence are separate mandatory inputs at the eventual composition boundary.
use std::{
    collections::BTreeMap,
    path::{Component, Path},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Browser,
    GentCanvas,
    MateCanvas,
}
impl Role {
    pub fn from_server(name: &str) -> Option<Self> {
        match name {
            "gent-browser" => Some(Self::Browser),
            "gent-canvas" => Some(Self::GentCanvas),
            "mate-canvas" => Some(Self::MateCanvas),
            _ => None,
        }
    }
}
fn absolute(p: &str) -> bool {
    let path = Path::new(p);
    path.is_absolute()
        && !path
            .components()
            .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
}
fn port(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 5
        && v.bytes().all(|b| b.is_ascii_digit())
        && v.parse::<u16>().is_ok_and(|n| n > 0)
}
/// Validate every selected environment key; callers must not filter unknowns.
/// HOME must equal the explicitly retained per-peer private owner directory.
/// Browser credentials stay in the private Start pipe and are never argv/files.
pub fn environment(role: Role, env: &BTreeMap<String, String>, private_home: &str) -> bool {
    absolute(private_home)
        && env.get("HOME").is_some_and(|v| v == private_home)
        && env.get("PATH").is_some_and(|v| v == "/usr/bin:/bin")
        && env.get("GENT_PORT").is_some_and(|v| port(v))
        && env.get("MATE_PORT").is_some_and(|v| port(v))
        && env.iter().all(|(k, v)| match k.as_str() {
            "PATH" | "HOME" | "GENT_PORT" | "MATE_PORT" => true,
            "GENT_BROWSER_TOKEN" => {
                role == Role::Browser
                    && !v.is_empty()
                    && v.len() <= 8192
                    && !v.chars().any(char::is_control)
            }
            _ => false,
        })
        && (role != Role::Browser || env.contains_key("GENT_BROWSER_TOKEN"))
}
/// Only a helper-owned sealed descriptor becomes the main module. No arbitrary
/// loaders, eval, workspace path or command flags enter this trusted profile.
pub fn arguments(entry_fd: i32) -> Option<Vec<String>> {
    (entry_fd >= 3).then(|| {
        vec![
            "--max-old-space-size=96".into(),
            "--preserve-symlinks-main".into(),
            format!("/proc/self/fd/{entry_fd}"),
        ]
    })
}
/// Host-selected source has no ambient HOME. The owner explicitly supplies its
/// private HOME and fixed PATH; every other selected key is preserved/rejected.
pub fn selected_environment(
    role: Role,
    selected: &BTreeMap<String, String>,
    private_home: &str,
) -> Option<BTreeMap<String, String>> {
    if selected.contains_key("HOME") {
        return None;
    }
    let mut env = selected.clone();
    if env.get("PATH").is_some_and(|v| v != "/usr/bin:/bin") {
        return None;
    }
    env.insert("PATH".into(), "/usr/bin:/bin".into());
    env.insert("HOME".into(), private_home.into());
    environment(role, &env, private_home).then_some(env)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn env() -> BTreeMap<String, String> {
        [
            ("PATH", "/usr/bin:/bin"),
            ("HOME", "/owned/private"),
            ("GENT_PORT", "4242"),
            ("MATE_PORT", "4242"),
            ("GENT_BROWSER_TOKEN", "owned-qa-only"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect()
    }
    #[test]
    fn token_role_is_explicit() {
        let mut e = env();
        assert!(environment(Role::Browser, &e, "/owned/private"));
        assert!(!environment(Role::GentCanvas, &e, "/owned/private"));
        e.remove("GENT_BROWSER_TOKEN");
        assert!(environment(Role::GentCanvas, &e, "/owned/private"));
        assert!(!environment(Role::Browser, &e, "/owned/private"));
    }
    #[test]
    fn no_ambient_loader_or_credential_carrier() {
        for key in [
            "NODE_OPTIONS",
            "NODE_PATH",
            "LD_PRELOAD",
            "PYTHONPATH",
            "ANTHROPIC_API_KEY",
        ] {
            let mut e = env();
            e.insert(key.into(), "qa".into());
            assert!(!environment(Role::Browser, &e, "/owned/private"));
        }
    }
    #[test]
    fn malformed_ports_and_private_home_rejected() {
        for value in ["0", "65536", "+123", "1\n", " 123", ""] {
            let mut e = env();
            e.insert("GENT_PORT".into(), value.into());
            assert!(!environment(Role::Browser, &e, "/owned/private"));
        }
        assert!(!environment(Role::Browser, &env(), "/other"));
    }
    #[test]
    fn only_helper_fd_arguments() {
        assert!(arguments(2).is_none());
        assert_eq!(
            arguments(3).unwrap(),
            [
                "--max-old-space-size=96",
                "--preserve-symlinks-main",
                "/proc/self/fd/3"
            ]
        );
        assert_eq!(Role::from_server("custom-node"), None);
    }
}
