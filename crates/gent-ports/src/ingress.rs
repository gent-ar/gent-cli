use gent_types::HostEpoch;

/// Whether a host currently admits write requests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngressMode {
    Open,
    Closed,
}

/// The durable ingress fence carried by every mutation request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostIngress {
    pub epoch: HostEpoch,
    pub mode: IngressMode,
}
