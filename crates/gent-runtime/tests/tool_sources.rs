use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, ToolSourceKind, ToolSourceRecord, WorkspaceRecord};

#[test]
fn coordinator_declarations_do_not_connect_to_tool_sources() {
    let coordinator =
        Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
    coordinator
        .create_workspace(&WorkspaceRecord {
            workspace_id: "workspace-a".into(),
            canonical_path: "/workspace".into(),
        })
        .unwrap();
    let source = ToolSourceRecord {
        tool_source_id: "source-a".into(),
        workspace_id: "workspace-a".into(),
        kind: ToolSourceKind::BuiltIn,
        source_name: "gent-git".into(),
        declared_tools: vec!["git:status".into()],
    };
    coordinator.create_tool_source(&source).unwrap();
    assert_eq!(
        coordinator.tool_sources("workspace-a").unwrap(),
        vec![source]
    );
}
