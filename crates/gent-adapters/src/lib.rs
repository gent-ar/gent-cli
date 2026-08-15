//! Home for public declarative provider manifests. No processes are started here.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterManifest {
    pub id: String,
    pub protocol_version: u16,
}
