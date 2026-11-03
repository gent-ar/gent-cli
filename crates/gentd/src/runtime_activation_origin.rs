use super::super::lease::{ActivationLease, LeaseFailure};
use gent_protocol::runtime_activation::{
    ActivationOriginLookup, ActivationOriginState as State, ActivationOriginStatus,
};

pub(super) fn inspect(value: &ActivationOriginLookup) -> ActivationOriginStatus {
    let mut result = ActivationOriginStatus {
        schema: 1,
        runtime_root: value.runtime_root.clone(),
        data_directory: value.data_directory.clone(),
        state: State::Unavailable,
        observed_host_epoch: None,
        observed_closed_ingress: None,
    };
    if value.expected_host_epoch == 0 {
        return result;
    }
    let (Ok(root), Ok(data)) = (
        super::status::plain_directory(&value.runtime_root),
        super::status::plain_directory(&value.data_directory),
    ) else {
        return result;
    };
    let held = match ActivationLease::existing_checked(&root, &data, None) {
        Ok(held) => held,
        Err(LeaseFailure::Busy) => {
            result.state = State::Busy;
            return result;
        }
        Err(_) => return result,
    };
    if held.root != root || held.data_dir != data {
        return result;
    }
    result.runtime_root = root;
    result.data_directory = data;
    result.observed_host_epoch = held.origin_host_epoch;
    result.observed_closed_ingress = held
        .origin_ingress
        .map(|i| i == gent_ports::IngressMode::Closed);
    result.state = if held.origin_host_epoch != Some(value.expected_host_epoch) {
        State::EpochMismatch
    } else if result.observed_closed_ingress == Some(true) {
        State::Closed
    } else {
        State::NotClosed
    };
    result
}
