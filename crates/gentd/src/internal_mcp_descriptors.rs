//! Provenance retained from the exact daemon injection, never inferred from callback names/paths.
use super::INTERNAL_SERVER_NAMES;
use serde_json::Value;
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InjectedInternalMcp {
    servers: Value,
}
impl InjectedInternalMcp {
    pub(crate) fn servers(&self) -> &Value {
        &self.servers
    }
    #[cfg(test)]
    pub(crate) fn fixture(root: &Path, cli: &str) -> Self {
        Self {
            servers: injected_servers_using(root, cli),
        }
    }
}
pub(super) fn capture(data_dir: &Path) -> Result<InjectedInternalMcp, String> {
    let cli = gent_cli_executable()?;
    Ok(InjectedInternalMcp {
        servers: injected_servers_using(data_dir, &cli),
    })
}
fn gent_cli_executable() -> Result<String, String> {
    let current = std::env::current_exe().map_err(|_| "Gent executable is unavailable")?;
    let directory = current.parent().ok_or("Gent executable is unavailable")?;
    let names = if cfg!(windows) {
        ["gent.exe", "gent-cli.exe"]
    } else {
        ["gent", "gent-cli"]
    };
    for directory in [Some(directory), directory.parent()].into_iter().flatten() {
        for name in names {
            let path = directory.join(name);
            if path.is_file() {
                return Ok(path.to_string_lossy().into_owned());
            }
        }
    }
    Err("Gent CLI executable is unavailable".into())
}
fn injected_servers_using(data_dir: &Path, executable: &str) -> Value {
    Value::Object(
        INTERNAL_SERVER_NAMES
            .into_iter()
            .map(|name| {
                let domain = name.trim_start_matches("gent-");
                (
                    name.to_owned(),
                    serde_json::json!({"command":executable,
            "args":["--data-dir",data_dir,"mcp",domain],"env":{}}),
                )
            })
            .collect::<serde_json::Map<String, Value>>(),
    )
}
