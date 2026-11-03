use gent_protocol::{DependencyProvider, PublicRunStartRequest};
use gent_types::HostEpoch;

pub(super) fn request(
    run_id: &str,
    coordinator_id: &str,
    host_epoch: HostEpoch,
) -> PublicRunStartRequest {
    PublicRunStartRequest {
        run_id: run_id.into(),
        coordinator_id: coordinator_id.into(),
        host_epoch,
        provider: DependencyProvider::Claude,
        executable: "daemon-resolved".into(),
        version: "daemon-resolved".into(),
        compatibility_entry: "daemon-resolved".into(),
    }
}
