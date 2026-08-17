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

    /// Returns whether a connector's complete declared tool set matches this registry.
    ///
    /// Declarations are compared as sets, so an executor cannot add a tool by repeating or
    /// reordering input. Invalid identifiers are never accepted as a registered declaration.
    #[must_use]
    pub fn matches_declaration(&self, connector: &str, declared_tools: &[String]) -> bool {
        let Ok(connector) = ConnectorId::parse(connector) else {
            return false;
        };
        let Some(definition) = self.connector(&connector) else {
            return false;
        };
        let declared = declared_tools
            .iter()
            .map(|tool| ToolName::parse(tool.clone()))
            .collect::<Result<std::collections::BTreeSet<_>, _>>();
        let Ok(declared) = declared else {
            return false;
        };
        let registered = definition
            .tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<std::collections::BTreeSet<_>>();
        declared.len() == declared_tools.len() && declared == registered
    }

    /// Produces a stable, credential-free declaration for a composition-time integrity digest.
    #[must_use]
    pub fn canonical_declaration(&self) -> String {
        self.connectors
            .values()
            .flat_map(|connector| {
                connector.tools.iter().map(move |tool| {
                    format!("{}\u{1f}{}\n", connector.id.as_str(), tool.name.as_str())
                })
            })
            .collect()
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
#[path = "registry_tests.rs"]
mod tests;
