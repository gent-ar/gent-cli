use crate::{claude_environment, sdk_launch::LaunchProfile};
use gent_drivers::{sandboxed_launch::SandboxedProviderLaunchError, supervisor::ProviderLaunch};
use gent_types::{SandboxNetworkPolicy, SandboxWorkspaceAccess, SandboxedLaunchRequest};
pub fn translate(
    request: &SandboxedLaunchRequest,
    launch: &ProviderLaunch,
) -> Result<LaunchProfile, SandboxedProviderLaunchError> {
    let reject = SandboxedProviderLaunchError::ProfileRejected;
    if request.lock != launch.lock
        || launch.provider != request.lock.provider
        || launch.executable.to_string_lossy() != request.lock.canonical_path
        || launch.workspace_root.as_deref() != Some(request.profile.workspace_root())
        || launch.permission_binding.as_ref() != request.profile.permission_binding()
    {
        return Err(reject);
    }
    // Managed provider generations may be user-owned. No mutable bytes execute:
    // the trusted gate must bind and seal the exact authorized ELF before GO.
    nix::unistd::access(
        std::path::Path::new(&request.lock.canonical_path),
        nix::unistd::AccessFlags::X_OK,
    )
    .map_err(|_| SandboxedProviderLaunchError::ProfileRejected)?;
    let current = gent_drivers::lock::rechecked_identity(&request.lock)
        .map_err(|_| SandboxedProviderLaunchError::LockChanged)?;
    if current != request.lock {
        return Err(SandboxedProviderLaunchError::LockChanged);
    }
    let state = request.profile.private_state().ok_or(reject.clone())?;
    if launch.run_id.as_deref() != Some(state.run_id())
        || !request.profile.inherited_environment().is_empty()
    {
        return Err(reject);
    }
    match (request.profile.network(), request.profile.gateway()) {
        (SandboxNetworkPolicy::Disabled, None) => {}
        (
            SandboxNetworkPolicy::ReviewedEgress {
                policy_digest_sha256,
            },
            Some(gateway),
        ) if gateway.policy_digest_sha256() == policy_digest_sha256
            && gateway.api() == gent_types::SandboxGatewayApi::Claude
            && request.lock.provider == "claude" => {}
        _ => return Err(reject),
    }
    let writable = request
        .profile
        .writable_roots()
        .contains(&request.profile.workspace_root().to_path_buf());
    if writable != matches!(launch.workspace_access, SandboxWorkspaceAccess::ReadWrite) {
        return Err(reject);
    }
    let text = |p: &std::path::Path| {
        p.to_str()
            .map(str::to_owned)
            .ok_or(SandboxedProviderLaunchError::ProfileRejected)
    };
    let mut readonly = request
        .profile
        .readable_roots()
        .iter()
        .map(|p| text(p))
        .collect::<Result<Vec<_>, _>>()?;
    let mut readwrite = request
        .profile
        .writable_roots()
        .iter()
        .map(|p| text(p))
        .collect::<Result<Vec<_>, _>>()?;
    for resource in request.profile.resources() {
        if resource.writable() {
            readwrite.push(text(resource.root())?);
        } else {
            readonly.push(text(resource.root())?);
        }
    }
    readonly.retain(|p| !readwrite.contains(p));
    if !readonly
        .iter()
        .any(|root| std::path::Path::new(&request.lock.canonical_path).starts_with(root))
    {
        return Err(SandboxedProviderLaunchError::ProfileRejected);
    }

    let mut profile = LaunchProfile {
        executable: request.lock.canonical_path.clone(),
        arguments: launch.arguments.clone(),
        cwd: text(request.profile.workspace_root())?,
        readonly,
        readwrite,
        denied: vec![],
        environment: vec![
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), text(&state.root().join("home"))?),
        ],
        timeout_ms: 5000,
    };
    if let Some(gateway) = request.profile.gateway() {
        profile
            .environment
            .push(("ANTHROPIC_BASE_URL".into(), gateway.origin()));
        profile
            .environment
            .push(("ANTHROPIC_AUTH_TOKEN".into(), gateway.scoped_token().into()));
        profile.environment.push((
            claude_environment::MCP_TOKEN_ENV.into(),
            gateway.scoped_token().into(),
        ));
        profile.environment.extend(
            claude_environment::FLAGS
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string())),
        );
        profile.environment.push((
            "CLAUDE_CONFIG_DIR".into(),
            text(&state.root().join("home/.claude"))?,
        ));
    }
    profile
        .validate()
        .map_err(|_| SandboxedProviderLaunchError::ProfileRejected)?;
    Ok(profile)
}
