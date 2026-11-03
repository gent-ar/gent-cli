use std::{io::Read, path::Path, time::Duration};

use super::{
    EgressError,
    codex_owner::CodexOwnerCredential,
    codex_probe::{CodexCredentialProbe, Progress, validate_home},
};
use crate::runtime_facade::model_catalog::probe::{ProbeStep, ProviderProbe};
use gent_drivers::PublicProvider;
use gent_types::RunVersionLock;

pub(super) fn resolve(
    lock: &RunVersionLock,
    workspace: &Path,
    resources: &crate::windows_provider_resources::ManagedProviderResources,
) -> Result<CodexOwnerCredential, EgressError> {
    if !resources.permits_codex_credentials(lock) {
        return Err(EgressError::Authentication);
    }
    if [
        "CODEX_API_KEY",
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "OPENAI_ORGANIZATION",
        "OPENAI_PROJECT",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some())
    {
        return Err(EgressError::Authentication);
    }
    let mut opening = gent_drivers::message_encoding::encode_codex_handshake(1)
        .map_err(|_| EgressError::Authentication)?;
    opening.push(CodexCredentialProbe::configuration_request()?);
    let mut probe = CodexCredentialProbe::new(workspace);
    let context = ProviderProbe {
        lifecycle_control: None,
        provider: PublicProvider::Codex,
        executable: lock.canonical_path.clone().into(),
        arguments: gent_drivers::launch_spec::codex_app_server_arguments(),
        timeout: Duration::from_secs(45),
        workspace_root: None,
    }
    .exchange(&opening, |value, process| {
        match probe
            .accept(value)
            .map_err(|_| "native credential metadata is unsupported")?
        {
            Progress::Continue => Ok(ProbeStep::Continue),
            Progress::RequestAccount(frame) => {
                process
                    .write_frame(&frame)
                    .map_err(|_| "native credential refresh request failed")?;
                Ok(ProbeStep::Continue)
            }
            Progress::Done(context) => Ok(ProbeStep::Done(context)),
        }
    })
    .map_err(|_| EgressError::Authentication)?;
    let home = validate_home(&context.home, workspace)?;
    let path = home.join("auth.json");
    if path.canonicalize().ok().as_ref() != Some(&path) {
        return Err(EgressError::Authentication);
    }
    let mut file = crate::windows_provider_resources::manifest::open_immutable(&path)
        .map_err(|_| EgressError::Authentication)?;
    if file
        .metadata()
        .map_err(|_| EgressError::Authentication)?
        .len()
        > 65_536
    {
        return Err(EgressError::Authentication);
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|_| EgressError::Authentication)?;
    context.credential(&bytes, crate::startup::unix_seconds())
}
