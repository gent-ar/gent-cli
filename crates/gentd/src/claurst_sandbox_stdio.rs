use crate::claurst_acp_transport::ClaurstAcpStdio;
use crate::claurst_local_runtime_owner::SystemClaurstAcpStdio;
use crate::sandbox_provider_process::RoutedProviderProcess;
use gent_drivers::interrupt::{ProcessTreeControl, ProcessTreeSignal};
use gent_drivers::{ProcessSandboxEvidence, ProviderProcess};

pub(crate) enum ClaurstOwnedAcpStdio {
    Host {
        process: SystemClaurstAcpStdio,
        fixed_mcp_roster: Option<String>,
    },
    Contained {
        process: RoutedProviderProcess,
        pending: Vec<u8>,
        fixed_mcp_roster: Option<String>,
        model_current: Box<dyn Fn() -> bool + Send>,
    },
}

impl ClaurstOwnedAcpStdio {
    fn settle_after_model_loss(&self) {
        if let Self::Contained {
            process,
            model_current,
            ..
        } = self
        {
            if !model_current() {
                let _ = process.signal_tree(ProcessTreeSignal::Terminate);
            }
        }
    }
}

impl ClaurstAcpStdio for ClaurstOwnedAcpStdio {
    fn fixed_mcp_roster_sha256(&self) -> Option<&str> {
        match self {
            Self::Contained {
                fixed_mcp_roster, ..
            }
            | Self::Host {
                fixed_mcp_roster, ..
            } => fixed_mcp_roster.as_deref(),
        }
    }
    fn sandbox_evidence(&self) -> Option<&ProcessSandboxEvidence> {
        self.settle_after_model_loss();
        match self {
            Self::Contained { process, .. } => process.sandbox_evidence(),
            Self::Host { .. } => None,
        }
    }
    fn write_frame(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.settle_after_model_loss();
        match self {
            Self::Host { process, .. } => process.write_frame(bytes),
            Self::Contained { process, .. } => process
                .write_frame(bytes)
                .map_err(|error| error.to_string()),
        }
    }
    fn try_read_frame(&mut self, maximum: usize) -> Result<Option<Vec<u8>>, String> {
        let Self::Contained {
            process, pending, ..
        } = self
        else {
            return match self {
                Self::Host { process, .. } => process.try_read_frame(maximum),
                Self::Contained { .. } => unreachable!(),
            };
        };
        loop {
            if let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
                if end + 1 > maximum {
                    return Err("contained ACP frame exceeded its limit".into());
                }
                return Ok(Some(pending.drain(..=end).collect()));
            }
            if pending.len() >= maximum {
                return Err("contained ACP frame exceeded its limit".into());
            }
            match process
                .next_stdout_chunk()
                .map_err(|error| error.to_string())?
            {
                Some(chunk) => pending.extend(chunk),
                None => return Ok(None),
            }
        }
    }
    fn exited(&mut self) -> Result<Option<String>, String> {
        self.settle_after_model_loss();
        match self {
            Self::Host { process, .. } => process.exited(),
            Self::Contained { process, .. } => process
                .try_exit_code()
                .map(|status| status.map(|code| format!("contained ACP exited: {code:?}")))
                .map_err(|error| error.to_string()),
        }
    }
}

impl Drop for ClaurstOwnedAcpStdio {
    fn drop(&mut self) {
        if let Self::Contained { process, .. } = self {
            let _ = process.signal_tree(ProcessTreeSignal::Terminate);
        }
    }
}
