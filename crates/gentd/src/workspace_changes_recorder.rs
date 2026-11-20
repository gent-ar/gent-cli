use std::path::Path;

use gent_git::workspace_changes::{changes_since, snapshot_tree};
use gent_store::SqliteLedger;
use gent_types::HostEpoch;

use crate::ordinary_lifecycle_control::OrdinaryLifecycleControl;

const TURN_PAGE: usize = 64;

pub(crate) fn capture_baseline(
    ledger: &SqliteLedger,
    conversation_id: &str,
    host_epoch: HostEpoch,
) -> Result<(), String> {
    if ledger
        .has_workspace_baseline(conversation_id)
        .map_err(|error| error.to_string())?
    {
        return Ok(());
    }
    let Some(workspace) = ledger
        .conversation_workspace_path(conversation_id)
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    let Ok(tree) = snapshot_tree(Path::new(&workspace)) else {
        return Ok(());
    };
    ledger
        .record_workspace_baseline(conversation_id, &tree, host_epoch)
        .map_err(|error| error.to_string())
}

pub(crate) fn record_after(ledger: &SqliteLedger, mut cursor: u64) -> Result<u64, String> {
    loop {
        let turns = ledger
            .workspace_change_turns(cursor, TURN_PAGE)
            .map_err(|error| error.to_string())?;
        if turns.is_empty() {
            return Ok(cursor);
        }
        for turn in turns {
            cursor = turn.projection_cursor;
            let Some(baseline) = &turn.baseline_tree else {
                continue;
            };
            let recorded = changes_since(Path::new(&turn.workspace_path), baseline)
                .map_err(|error| error.to_string())
                .and_then(|changes| {
                    ledger
                        .record_workspace_changes(&turn, changes.files, changes.truncated)
                        .map_err(|error| error.to_string())
                });
            if let Err(error) = recorded {
                eprintln!(
                    "workspace changes for turn {} were not recorded: {error}",
                    turn.turn_id
                );
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceChangesRecorder {
    ledger: SqliteLedger,
}

impl WorkspaceChangesRecorder {
    pub(crate) fn new(ledger: SqliteLedger) -> Self {
        Self { ledger }
    }

    pub(crate) async fn run(self, control: OrdinaryLifecycleControl) {
        if control.wait_until_ready().await.is_err() {
            return;
        }
        let mut commits = self.ledger.commits();
        let Ok(mut cursor) = self.ledger.agent_chat_projection_head() else {
            eprintln!("workspace changes recorder could not read the projection head");
            return;
        };
        loop {
            tokio::select! {
                () = control.shutdown_requested() => return,
                changed = commits.changed() => if changed.is_err() { return },
            }
            let ledger = self.ledger.clone();
            match tokio::task::spawn_blocking(move || record_after(&ledger, cursor)).await {
                Ok(Ok(next)) => cursor = next,
                Ok(Err(error)) => eprintln!("workspace changes were not recorded: {error}"),
                Err(_) => eprintln!("workspace changes task failed"),
            }
        }
    }
}

#[cfg(test)]
#[path = "workspace_changes_recorder_tests.rs"]
mod tests;
