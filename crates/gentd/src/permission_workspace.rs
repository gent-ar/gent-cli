//! Daemon-local durable namespace for user permission preferences.

use gent_ports::WorkspaceLedger;
use gent_store::SqliteLedger;
use gent_types::WorkspaceRecord;

/// The local runtime settings are intentionally independent from Git workspaces.
pub(crate) const SETTINGS_WORKSPACE_ID: &str = "gent-local-settings";

/// Creates the one local settings identity exactly once; no provider or filesystem effect occurs.
pub(crate) fn ensure(ledger: &SqliteLedger, data_dir: &std::path::Path) -> Result<(), String> {
    if ledger
        .find_workspace(SETTINGS_WORKSPACE_ID)
        .map_err(|error| error.to_string())?
        .is_none()
    {
        ledger
            .create_workspace(&WorkspaceRecord {
                workspace_id: SETTINGS_WORKSPACE_ID.into(),
                canonical_path: data_dir
                    .canonicalize()
                    .map_err(|error| error.to_string())?
                    .to_string_lossy()
                    .into_owned(),
            })
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
