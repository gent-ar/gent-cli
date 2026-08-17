use super::{
    ConnectorDefinition, ConnectorId, IdentifierError, QualifiedToolId, RegistryError,
    ToolDefinition, ToolName, ToolRegistry,
};

fn connector(id: &str, tools: &[&str]) -> ConnectorDefinition {
    ConnectorDefinition {
        id: ConnectorId::parse(id).unwrap(),
        tools: tools
            .iter()
            .map(|name| ToolDefinition {
                name: ToolName::parse(*name).unwrap(),
            })
            .collect(),
    }
}

#[test]
fn registry_indexes_tools_by_connector_and_name() {
    let registry = ToolRegistry::build(vec![connector("github", &["issues"])]).unwrap();
    let id = QualifiedToolId::new(
        ConnectorId::parse("github").unwrap(),
        ToolName::parse("issues").unwrap(),
    );
    assert_eq!(registry.connector_count(), 1);
    assert_eq!(registry.tool_count(), 1);
    assert_eq!(registry.tool(&id).unwrap().qualified_id(), id);
}

#[test]
fn duplicate_tools_are_rejected_before_registry_mutates() {
    let error = ToolRegistry::build(vec![connector("github", &["issues", "issues"])]).unwrap_err();
    assert!(matches!(error, RegistryError::DuplicateTool(_)));
}

#[test]
fn duplicate_connectors_are_rejected() {
    let error =
        ToolRegistry::build(vec![connector("github", &[]), connector("github", &[])]).unwrap_err();
    assert!(matches!(error, RegistryError::DuplicateConnector(_)));
}

#[test]
fn unsafe_or_ambiguous_identifiers_are_rejected() {
    assert_eq!(
        ConnectorId::parse("GitHub"),
        Err(IdentifierError::InvalidConnectorId)
    );
    assert_eq!(
        ToolName::parse("../shell"),
        Err(IdentifierError::InvalidToolName)
    );
}

#[test]
fn declarations_are_complete_and_canonical() {
    let registry = ToolRegistry::build(vec![connector("github", &["issues", "pulls"])]).unwrap();
    assert!(registry.matches_declaration("github", &["pulls".into(), "issues".into()]));
    assert!(!registry.matches_declaration("github", &["issues".into()]));
    assert!(!registry.matches_declaration("github", &["issues".into(), "issues".into()]));
    assert_eq!(
        registry.canonical_declaration(),
        "github\u{1f}issues\ngithub\u{1f}pulls\n"
    );
}
