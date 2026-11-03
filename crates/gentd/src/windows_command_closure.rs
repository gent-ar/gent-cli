use super::manifest::{self, ProviderResources, ResourceRoot};
use serde::Deserialize;

const COMMAND_TOOLS_JSON: &str = include_str!("../windows-command-tools.json");

#[derive(Deserialize)]
struct CommandTools {
    tools: Vec<String>,
    libraries: Vec<String>,
    #[serde(rename = "gentTools")]
    gent_tools: Vec<String>,
    licenses: Vec<String>,
}

pub(crate) fn required_paths() -> Result<Vec<String>, String> {
    let closure: CommandTools = serde_json::from_str(COMMAND_TOOLS_JSON)
        .map_err(|_| "the reviewed command tool list is invalid".to_owned())?;
    Ok(closure
        .tools
        .iter()
        .chain(&closure.libraries)
        .chain(&closure.gent_tools)
        .map(|name| format!("usr/bin/{name}"))
        .chain(closure.licenses)
        .collect())
}

pub(crate) fn require_command_closure(contract: &ProviderResources) -> Result<(), String> {
    manifest::require_command_shims(contract)?;
    for path in required_paths()? {
        if !contract
            .files
            .iter()
            .any(|file| file.root == ResourceRoot::GitRuntime && file.path == path)
        {
            return Err(
                "the reviewed command tool closure is incomplete; repair the runtime bundle".into(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::manifest::ResourceFile;
    use super::*;

    fn row(root: ResourceRoot, path: &str) -> ResourceFile {
        ResourceFile {
            root,
            path: path.to_owned(),
            sha256: "d".repeat(64),
        }
    }

    fn contract(skip: Option<&str>) -> ProviderResources {
        let mut files: Vec<_> = required_paths()
            .unwrap()
            .iter()
            .filter(|path| Some(path.as_str()) != skip)
            .map(|path| row(ResourceRoot::GitRuntime, path))
            .collect();
        files.extend(
            manifest::COMMAND_SHIMS
                .iter()
                .map(|shim| row(ResourceRoot::PowerShellLauncher, shim)),
        );
        ProviderResources {
            provider: "claude".into(),
            platform: "win32-x64".into(),
            package_name: None,
            package_version: None,
            package_integrity: None,
            node_sha256: None,
            executable_sha256: None,
            credential_contract: "claude-loopback-v1".into(),
            files,
            os_libraries: Vec::new(),
        }
    }

    #[test]
    fn every_listed_tool_library_and_license_is_required_by_digest_row() {
        assert!(require_command_closure(&contract(None)).is_ok());
        for path in [
            "usr/bin/sed.exe",
            "usr/bin/xargs.exe",
            "usr/bin/msys-pcre-1.dll",
            "usr/bin/git-remote-gent.exe",
            "usr/bin/npm",
            "usr/bin/npx.cmd",
            "LICENSE.txt",
        ] {
            assert!(
                require_command_closure(&contract(Some(path))).is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn listed_tools_cover_the_command_contract() {
        let paths = required_paths().unwrap();
        for tool in [
            "sed", "find", "head", "tail", "sort", "cut", "xargs", "tr", "wc", "mkdir", "rm", "cp",
            "mv", "env", "ls", "grep", "cat",
        ] {
            assert!(paths.contains(&format!("usr/bin/{tool}.exe")), "{tool}");
        }
    }
}
