use gent_ports::PublicProviderRunError;

pub(super) fn selected_config(
    configured: Option<&std::path::Path>,
    names: &[String],
    run_id: &str,
) -> Result<Option<std::path::PathBuf>, PublicProviderRunError> {
    if names.is_empty() {
        return Ok(configured.map(std::path::Path::to_path_buf));
    }
    let path = configured.ok_or_else(|| {
        PublicProviderRunError::Failed("selected MCP source has no configured server".into())
    })?;
    let metadata = std::fs::metadata(path)
        .map_err(|_| PublicProviderRunError::Failed("Claude MCP config is unavailable".into()))?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err(PublicProviderRunError::Failed(
            "Claude MCP config is invalid".into(),
        ));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|_| {
            PublicProviderRunError::Failed("Claude MCP config is unavailable".into())
        })?)
        .map_err(|_| PublicProviderRunError::Failed("Claude MCP config is invalid".into()))?;
    let servers = value
        .get("mcpServers")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| PublicProviderRunError::Failed("Claude MCP config is invalid".into()))?;
    let mut selected = serde_json::Map::new();
    for name in names {
        selected.insert(
            name.clone(),
            servers.get(name).cloned().ok_or_else(|| {
                PublicProviderRunError::Failed("selected MCP source is not configured".into())
            })?,
        );
    }
    let _ = run_id;
    let output =
        std::env::temp_dir().join(format!("gent-claude-mcp-{}.json", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec(&serde_json::json!({"mcpServers": selected})).map_err(|_| {
        PublicProviderRunError::Failed("selected Claude MCP config is invalid".into())
    })?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .map_err(|_| {
            PublicProviderRunError::Failed("selected Claude MCP config is unavailable".into())
        })?;
    file.write_all(&bytes).map_err(|_| {
        PublicProviderRunError::Failed("selected Claude MCP config is unavailable".into())
    })?;
    Ok(Some(output))
}
