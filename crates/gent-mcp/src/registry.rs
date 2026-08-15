//! Immutable connector and tool registry validation.

use std::collections::BTreeMap;

/// A validated connector identifier.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ConnectorId(String);

impl ConnectorId {
    /// Parses a stable connector identifier.
    ///
    /// # Errors
    /// Returns an error unless the identifier is lowercase ASCII and contains
    /// only letters, digits, `.`, `_`, or `-`.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        valid_identifier(&value)
            .then_some(Self(value))
            .ok_or(IdentifierError::InvalidConnectorId)
    }

    /// Returns the canonical connector identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated tool name scoped to one connector.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ToolName(String);

impl ToolName {
    /// Parses a connector-local tool name.
    ///
    /// # Errors
    /// Returns an error unless the name is lowercase ASCII and contains only
    /// letters, digits, `.`, `_`, or `-`.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        valid_identifier(&value)
            .then_some(Self(value))
            .ok_or(IdentifierError::InvalidToolName)
    }

    /// Returns the canonical tool name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One tool declared by a connector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolDefinition {
    pub name: ToolName,
}

/// One connector and the tools it is allowed to expose.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectorDefinition {
    pub id: ConnectorId,
    pub tools: Vec<ToolDefinition>,
}

/// Globally unambiguous MCP tool identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct QualifiedToolId {
    connector: ConnectorId,
    tool: ToolName,
}

impl QualifiedToolId {
    /// Creates a globally unique identity from validated component identifiers.
    #[must_use]
    pub fn new(connector: ConnectorId, tool: ToolName) -> Self {
        Self { connector, tool }
    }

    /// Returns the connector portion of this identifier.
    #[must_use]
    pub fn connector(&self) -> &ConnectorId {
        &self.connector
    }

    /// Returns the connector-local tool portion of this identifier.
    #[must_use]
    pub fn tool(&self) -> &ToolName {
        &self.tool
    }
}

/// Indexed registry entry that names its connector and tool without credentials.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolReference {
    pub connector: ConnectorId,
    pub tool: ToolName,
}

impl ToolReference {
    /// Returns the stable global identity for this entry.
    #[must_use]
    pub fn qualified_id(&self) -> QualifiedToolId {
        QualifiedToolId::new(self.connector.clone(), self.tool.clone())
    }
}

/// Read-only connector registry with an explicit global tool index.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolRegistry {
    connectors: BTreeMap<ConnectorId, ConnectorDefinition>,
    tools: BTreeMap<QualifiedToolId, ToolReference>,
}

impl ToolRegistry {
    /// Validates connector and tool uniqueness, then builds an immutable index.
    ///
    /// # Errors
    /// Returns an error when a connector or a connector-local tool is duplicated.
    pub fn build(connectors: Vec<ConnectorDefinition>) -> Result<Self, RegistryError> {
        let mut registry = Self::default();
        for connector in connectors {
            registry.insert(connector)?;
        }
        Ok(registry)
    }

    /// Returns a connector definition by stable identifier.
    #[must_use]
    pub fn connector(&self, id: &ConnectorId) -> Option<&ConnectorDefinition> {
        self.connectors.get(id)
    }

    /// Returns one validated global tool entry.
    #[must_use]
    pub fn tool(&self, id: &QualifiedToolId) -> Option<&ToolReference> {
        self.tools.get(id)
    }

    /// Returns connector count without exposing mutable registry state.
    #[must_use]
    pub fn connector_count(&self) -> usize {
        self.connectors.len()
    }

    /// Returns tool count without exposing mutable registry state.
    #[must_use]
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    fn insert(&mut self, connector: ConnectorDefinition) -> Result<(), RegistryError> {
        if self.connectors.contains_key(&connector.id) {
            return Err(RegistryError::DuplicateConnector(connector.id));
        }
        let mut pending = Vec::with_capacity(connector.tools.len());
        for definition in &connector.tools {
            let reference = ToolReference {
                connector: connector.id.clone(),
                tool: definition.name.clone(),
            };
            let id = reference.qualified_id();
            if self.tools.contains_key(&id) || pending.contains(&id) {
                return Err(RegistryError::DuplicateTool(id));
            }
            pending.push(id);
        }
        for id in pending {
            self.tools.insert(
                id.clone(),
                ToolReference {
                    connector: id.connector,
                    tool: id.tool,
                },
            );
        }
        self.connectors.insert(connector.id.clone(), connector);
        Ok(())
    }
}

/// Identifier construction errors.
#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum IdentifierError {
    #[error("connector identifiers must be lowercase ASCII tokens")]
    InvalidConnectorId,
    #[error("tool names must be lowercase ASCII tokens")]
    InvalidToolName,
}

/// Registry construction errors.
#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum RegistryError {
    #[error("duplicate connector: {0:?}")]
    DuplicateConnector(ConnectorId),
    #[error("duplicate connector-local tool: {0:?}")]
    DuplicateTool(QualifiedToolId),
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

#[cfg(test)]
mod tests {
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
        let error =
            ToolRegistry::build(vec![connector("github", &["issues", "issues"])]).unwrap_err();
        assert!(matches!(error, RegistryError::DuplicateTool(_)));
    }

    #[test]
    fn duplicate_connectors_are_rejected() {
        let error = ToolRegistry::build(vec![connector("github", &[]), connector("github", &[])])
            .unwrap_err();
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
}
