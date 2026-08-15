//! Home for public declarative provider manifests. No processes are started here.

pub mod compatibility;
pub mod compatibility_cache;
pub mod manifest;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterManifest {
    pub id: String,
    pub protocol_version: u16,
}
