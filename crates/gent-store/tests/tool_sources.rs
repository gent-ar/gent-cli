use gent_ports::{ToolSourceLedger, WorkspaceLedger};
use gent_store::SqliteLedger;
use gent_types::{ToolSourceKind, ToolSourceRecord, WorkspaceRecord};

fn workspace() -> WorkspaceRecord {
    WorkspaceRecord {
        workspace_id: "workspace-a".into(),
        canonical_path: "/workspace".into(),
    }
}

fn source(name: &str) -> ToolSourceRecord {
    ToolSourceRecord {
        tool_source_id: format!("source-{name}"),
        workspace_id: "workspace-a".into(),
        kind: ToolSourceKind::McpServer,
        source_name: name.into(),
        declared_tools: vec!["mcp/search".into(), "mcp/summarize".into()],
    }
}

#[test]
fn tool_sources_are_durable_workspace_declarations() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    ledger.create_workspace(&workspace()).unwrap();
    ledger.create_tool_source(&source("search")).unwrap();
    drop(ledger);

    assert_eq!(
        SqliteLedger::open(path)
            .unwrap()
            .list_tool_sources("workspace-a")
            .unwrap(),
        vec![source("search")]
    );
}

#[test]
fn tool_sources_reject_missing_workspace_and_noncanonical_tools() {
    let ledger = SqliteLedger::in_memory().unwrap();
    assert!(ledger.create_tool_source(&source("search")).is_err());
    ledger.create_workspace(&workspace()).unwrap();
    assert!(
        ledger
            .create_tool_source(&ToolSourceRecord {
                declared_tools: vec!["mcp/summarize".into(), "mcp/search".into()],
                ..source("search")
            })
            .is_err()
    );
    assert!(
        ledger
            .create_tool_source(&ToolSourceRecord {
                source_name: "not a source".into(),
                ..source("invalid")
            })
            .is_err()
    );
}
