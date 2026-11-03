use super::{ClaurstAcpStdio, ClaurstAcpTransport};
use gent_types::{HostEpoch, PolicyRecord, SandboxEnforcement};

impl<S: ClaurstAcpStdio> ClaurstAcpTransport<S> {
    pub(crate) fn supports_images(&self) -> bool {
        self.supports_images
    }

    pub(crate) fn is_idle(&self) -> bool {
        self.pending_prompt_id.is_none()
    }

    pub(crate) fn exited(&mut self) -> Result<Option<String>, String> {
        self.stdio.exited()
    }

    pub(crate) fn permission_enforcement(
        &mut self,
        run: &str,
        policy: &PolicyRecord,
        epoch: HostEpoch,
    ) -> SandboxEnforcement {
        if !matches!(self.stdio.exited(), Ok(None)) {
            return SandboxEnforcement::Unavailable;
        }
        let valid = self
            .stdio
            .sandbox_evidence()
            .filter(|proof| proof.run_id() == run)
            .and_then(|proof| proof.permission_binding())
            .is_some_and(|binding| {
                binding.run_id() == run
                    && binding.policy() == policy
                    && binding.host_epoch() == epoch
            });
        if valid {
            SandboxEnforcement::Enforced
        } else {
            SandboxEnforcement::Unavailable
        }
    }
}
