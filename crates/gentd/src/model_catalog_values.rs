use std::time::{Duration, Instant};

use gent_protocol::model_catalog::{CatalogModel, ProviderAvailability};
use gent_types::{AgentChatEffort, ProviderAuthLifecycle, ProviderAuthProvider};

use crate::{
    provider_auth_api::{ProviderAuthPort, STATUS_SETTLE_BOUND},
    provider_launch_budget::ProviderLaunchError,
};

pub(crate) const PROVIDER_DEFAULT_MODEL: &str = "default";

pub(crate) fn effort(value: &str) -> Option<AgentChatEffort> {
    match value {
        "low" => Some(AgentChatEffort::Low),
        "medium" => Some(AgentChatEffort::Medium),
        "high" => Some(AgentChatEffort::High),
        "xhigh" => Some(AgentChatEffort::XHigh),
        "max" => Some(AgentChatEffort::Max),
        "ultra" => Some(AgentChatEffort::Ultra),
        _ => None,
    }
}

pub(crate) fn provider_default_model() -> CatalogModel {
    CatalogModel {
        id: PROVIDER_DEFAULT_MODEL.into(),
        label: "Default".into(),
        description: None,
        is_default: true,
        efforts: Vec::new(),
        default_effort: None,
        local: None,
    }
}

pub(crate) fn public_availability(
    auth: &dyn ProviderAuthPort,
    provider: ProviderAuthProvider,
) -> Result<ProviderAvailability, ProviderLaunchError> {
    let deadline = Instant::now() + STATUS_SETTLE_BOUND;
    let mut lifecycle = auth.check_status(provider).lifecycle;
    while lifecycle == ProviderAuthLifecycle::Checking && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
        lifecycle = auth.check_status(provider).lifecycle;
    }
    match lifecycle {
        ProviderAuthLifecycle::Checking => Err(ProviderLaunchError::TimedOut(
            "the provider account is still being checked".into(),
        )),
        ProviderAuthLifecycle::NotInstalled => Ok(ProviderAvailability::NotInstalled),
        ProviderAuthLifecycle::Authenticated => Ok(ProviderAvailability::Ready),
        ProviderAuthLifecycle::Failed | ProviderAuthLifecycle::ProviderChanged => Err(
            ProviderLaunchError::Failed("the provider executable could not be verified".into()),
        ),
        ProviderAuthLifecycle::Unauthenticated
        | ProviderAuthLifecycle::ChallengeOffered
        | ProviderAuthLifecycle::Verifying
        | ProviderAuthLifecycle::Expired
        | ProviderAuthLifecycle::Cancelled
        | ProviderAuthLifecycle::TimedOut => Ok(ProviderAvailability::SignedOut),
    }
}
