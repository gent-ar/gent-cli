use gent_ports::{AgentChatReadLedger, PackageInstallPolicy};
use gent_protocol::PromptProviderProvisionFrame;
use gent_runtime::AgentChatReadService;
use gent_types::{
    ProviderPromptProvisionBinding, ProviderPromptProvisionCommandBinding,
    ProviderPromptProvisionPackageBinding,
};

use crate::{
    authority_clock::AuthorityClock, dependency_catalog::DependencyCatalog,
    private_provider_review::install_review,
};

use super::values::{Derived, public_provider};

pub(super) fn request<L, P, C>(
    reads: &AgentChatReadService<L>,
    catalog: &DependencyCatalog,
    policy: &P,
    clock: &C,
    frame: PromptProviderProvisionFrame,
) -> Result<Derived, String>
where
    L: AgentChatReadLedger,
    P: PackageInstallPolicy,
    C: AuthorityClock,
{
    let PromptProviderProvisionFrame::Confirm {
        receipt_id,
        idempotency_key,
        host_epoch,
        prompt_receipt_id,
        conversation_id,
        run_id,
        consent_granted,
        reviewed_plan_digest,
    } = frame
    else {
        return Err("prompt provider provision result frames are server-only".into());
    };
    let detail = reads
        .detail(&conversation_id.0)
        .map_err(|error| error.to_string())?;
    if detail.current_run_id != run_id.0 {
        return Err(
            "staleAgentChatRun: refresh conversation detail before confirming provision".into(),
        );
    }
    let selection = detail
        .runs
        .into_iter()
        .find(|run| run.run_id == run_id.0)
        .map(|run| run.selection)
        .ok_or_else(|| "agent-chat current run is absent from its conversation".to_owned())?;
    let provider = public_provider(selection.provider)?;
    let review = install_review(catalog, policy, clock, provider)?;
    let binding = ProviderPromptProvisionCommandBinding {
        prompt: ProviderPromptProvisionBinding {
            prompt_receipt_id,
            conversation_id,
            run_id,
            provider: provider.as_str().into(),
            action: "install".into(),
            consent_granted,
            reviewed_plan_digest,
        },
        expected_reviewed_plan_digest: review.reviewed_plan_digest,
        package: ProviderPromptProvisionPackageBinding {
            provider: provider.as_str().into(),
            package_name: review.package.package_name,
            version: review.package.version,
            integrity: review.package.integrity,
            package_policy_digest_sha256: review.package.package_policy_digest_sha256,
        },
    };
    binding
        .is_valid()
        .then_some(Derived {
            receipt_id,
            idempotency_key,
            host_epoch,
            binding,
        })
        .ok_or_else(|| "daemon-derived prompt provision binding is invalid".into())
}
