//! MCP domain policy and registry validation.
//!
//! This crate is deliberately free of process and network implementations.
//! `gentd` may compose its pure transitions with future transport ports only
//! after the relevant authority gate has passed.

pub mod lifecycle;
pub mod registry;

pub use lifecycle::{
    McpEffect, McpEvent, McpMode, McpState, McpTransition, initial_state, transition,
};
pub use registry::{
    ConnectorDefinition, ConnectorId, IdentifierError, QualifiedToolId, RegistryError,
    ToolDefinition, ToolName, ToolReference, ToolRegistry,
};
