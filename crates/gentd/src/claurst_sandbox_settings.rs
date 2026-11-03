use crate::claurst_local_runtime::LocalProcessLaunch;

pub(super) fn packaged_shells() -> Vec<(String, String)> {
    let Some(windows) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("runtime").join("windows")))
    else {
        return Vec::new();
    };
    [
        (
            "GENT_GIT_BASH_PATH",
            windows.join("git").join("usr").join("bin").join("bash.exe"),
        ),
        (
            "GENT_POWERSHELL_PATH",
            windows.join("powershell").join("pwsh.exe"),
        ),
    ]
    .into_iter()
    .filter(|(_, path)| path.is_file())
    .map(|(name, path)| (name.to_owned(), path.display().to_string()))
    .collect()
}

pub(super) fn isolated_settings(
    launch: &LocalProcessLaunch,
) -> Result<
    (
        std::path::PathBuf,
        std::path::PathBuf,
        String,
        Option<String>,
    ),
    String,
> {
    let home = std::path::PathBuf::from(
        launch
            .environment
            .get("CLAURST_HOME")
            .ok_or("Claurst isolated home missing")?,
    );
    let settings = home.join("settings.json");
    if std::fs::metadata(&settings)
        .map_err(|_| "Claurst isolated settings missing")?
        .len()
        > 1024 * 1024
    {
        return Err("Claurst settings exceed launch limit".into());
    }
    let contents =
        std::fs::read_to_string(&settings).map_err(|_| "Claurst isolated settings unavailable")?;
    let settings_value: serde_json::Value =
        serde_json::from_str(&contents).map_err(|_| "Claurst isolated settings are invalid")?;
    let mcp_servers = settings_value
        .pointer("/config/mcp_servers")
        .and_then(|v| v.as_array())
        .ok_or("Claurst MCP roster missing")?;
    let fixed_mcp_roster = if mcp_servers.is_empty() {
        None
    } else {
        Some(
            crate::claurst_acp_transport::mcp_roster_sha256(mcp_servers)
                .map_err(|_| "Claurst MCP roster invalid")?,
        )
    };
    Ok((home, settings, contents, fixed_mcp_roster))
}

#[cfg(test)]
#[path = "claurst_sandbox_settings_tests.rs"]
mod tests;
