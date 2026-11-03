//! Reviewed Windows MCP launch shapes. No shell, loader flags or directory entrypoint.
use super::mcp::Error;
use std::{collections::BTreeMap, path::Path};

pub enum Profile {
    Gent,
    Node,
}
fn injection(key: &str) -> bool {
    matches!(
        key.to_ascii_uppercase().as_str(),
        "NODE_OPTIONS"
            | "NODE_PATH"
            | "PYTHONPATH"
            | "PYTHONHOME"
            | "RUBYOPT"
            | "PERL5OPT"
            | "PERL5LIB"
            | "JAVA_TOOL_OPTIONS"
            | "JDK_JAVA_OPTIONS"
            | "_JAVA_OPTIONS"
            | "COMPLUS_ENABLE_PROFILING"
            | "COR_ENABLE_PROFILING"
            | "COR_PROFILER"
            | "COR_PROFILER_PATH"
            | "DOTNET_STARTUP_HOOKS"
            | "LD_PRELOAD"
            | "LD_LIBRARY_PATH"
            | "DYLD_INSERT_LIBRARIES"
            | "DYLD_LIBRARY_PATH"
    )
}
pub fn validate(
    command: &Path,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> Result<Profile, Error> {
    if env.keys().any(|key| injection(key)) {
        return Err(Error::Denied);
    }
    // Files containing credential carriers cannot be secret-filtered without a
    // separate explicit capture contract. None of the seven current profiles use them.
    if args.iter().any(|s| {
        let s = s.to_ascii_lowercase();
        s.starts_with("--headers-file")
            || s.starts_with("--token-file")
            || s.starts_with("--password-file")
            || s.starts_with("--secret-file")
    }) {
        return Err(Error::Denied);
    }
    let name = command
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(Error::Protocol)?;
    if name.eq_ignore_ascii_case("node.exe") {
        let entry = args.first().map(Path::new).ok_or(Error::Protocol)?;
        if !entry.is_absolute()
            || !entry.is_file()
            || !entry.extension().and_then(|v| v.to_str()).is_some_and(|e| {
                ["js", "mjs", "cjs"]
                    .iter()
                    .any(|wanted| e.eq_ignore_ascii_case(wanted))
            })
        {
            return Err(Error::Denied);
        }
        // Node stops parsing runtime flags at this pinned regular-file entrypoint;
        // subsequent strings belong to the reviewed server's data argument contract.
        return Ok(Profile::Node);
    }
    if name.eq_ignore_ascii_case("gent.exe")
        && matches!(args.len(), 4 | 6)
        && args[0] == "--data-dir"
        && Path::new(&args[1]).is_absolute()
        && args[2] == "mcp"
        && ["automations", "browser", "canvas", "chat", "forge", "goal"].contains(&args[3].as_str())
        && (args.len() == 4
            || (args[4] == "--conversation-id" && !args[5].is_empty() && args[5].len() <= 512))
    {
        return Ok(Profile::Gent);
    }
    Err(Error::Denied)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mcp_profile_node_loader_flags_directory_and_env_injection_are_denied() {
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("server.js");
        std::fs::write(&script, b"// pinned fixture").unwrap();
        let command = std::path::PathBuf::from("C:/owned/node.exe");
        assert!(
            validate(
                &command,
                &[
                    script.to_string_lossy().into_owned(),
                    "--public-data".into()
                ],
                &BTreeMap::new()
            )
            .is_ok()
        );
        for first in [
            "--require=workspace/evil.js".to_string(),
            "--eval".into(),
            root.path().to_string_lossy().into_owned(),
            "relative.js".into(),
        ] {
            assert!(validate(&command, &[first], &BTreeMap::new()).is_err());
        }
        for key in [
            "NODE_OPTIONS",
            "NODE_PATH",
            "PYTHONPATH",
            "DOTNET_STARTUP_HOOKS",
        ] {
            assert!(
                validate(
                    &command,
                    &[script.to_string_lossy().into_owned()],
                    &BTreeMap::from([(key.into(), "private".into())])
                )
                .is_err()
            );
        }
    }
    #[test]
    fn mcp_profile_gent_is_exact_without_shell_or_loader_fallback() {
        let args = vec![
            "--data-dir".into(),
            "C:/host/data".into(),
            "mcp".into(),
            "chat".into(),
        ];
        assert!(validate(Path::new("C:/owned/gent.exe"), &args, &BTreeMap::new()).is_ok());
        assert!(
            validate(
                Path::new("C:/Windows/System32/cmd.exe"),
                &args,
                &BTreeMap::new()
            )
            .is_err()
        );
        let mut bad = args.clone();
        bad[2] = "run".into();
        assert!(validate(Path::new("C:/owned/gent.exe"), &bad, &BTreeMap::new()).is_err());
    }
}
