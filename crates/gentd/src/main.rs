mod api;
mod attachment_api;
mod attachment_transport;
mod compatibility_assessment;
mod dependency_actions;
mod dependency_catalog;
#[cfg(test)]
mod dependency_catalog_tests;
mod event_stream;
mod host_lock;
mod public_runs;
mod transport;
#[cfg(test)]
mod transport_event_tests;
#[cfg(test)]
mod transport_stream_tests;
#[cfg(test)]
mod transport_tests;
#[cfg(test)]
mod transport_timeline_tests;
#[cfg(windows)]
mod transport_windows;
#[cfg(all(test, windows))]
mod transport_windows_tests;

use std::path::PathBuf;

use clap::Parser;
use gent_core::{DecisionCommandOutcome, DecisionEvidence as CoreDecisionEvidence};
use gent_drivers::installer::SystemDependencyInstaller;
use gent_protocol::{
    AttachmentFrame, DecisionEvidence, DecisionSubmission, DependencyActionRequest,
    DependencyActionResult, DependencyPlan, DependencyPlanRequest,
};
use gent_runtime::catalog::validate_observed_capabilities;
use gent_runtime::{AttachmentService, Coordinator, DependencyActionService};
use gent_store::{FileAttachmentBlobs, SqliteLedger};
use gent_types::{
    CapabilitySet, Command, ConversationStatus, ConversationTimeline, DecisionCommand,
    DecisionSettlement, DoctorReport, EventResume, HostStatus, Receipt,
};
#[cfg(unix)]
use tokio::net::UnixListener;

use crate::compatibility_assessment::CompatibilityAssessment;
use crate::dependency_actions::SystemDependencyExecutor;
use crate::dependency_catalog::DependencyCatalog;
use crate::public_runs::{DaemonPublicRuns, observer_service};

#[derive(Debug, Parser)]
#[command(name = "gentd", about = "Gent's local runtime host")]
struct Args {
    /// Directory containing the local IPC endpoint and durable `SQLite` ledger.
    #[arg(long, env = "GENT_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Explicit Unix socket path, primarily for supervised Unix launches and tests.
    #[cfg(unix)]
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Read-only cache of a previously verified signed compatibility manifest.
    #[arg(long, env = "GENT_COMPATIBILITY_CACHE")]
    compatibility_cache: Option<PathBuf>,
    /// Trusted public key as `key-id:lowercase-hex`; may be passed more than once.
    #[arg(long = "compatibility-key", env = "GENT_COMPATIBILITY_KEY")]
    compatibility_keys: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let data_dir = args.data_dir.clone().unwrap_or_else(default_data_dir);
    std::fs::create_dir_all(&data_dir)?;
    let _host_lock = host_lock::acquire(&data_dir)?;
    let observed_capabilities = transport::observed_capabilities();
    let compatibility = CompatibilityAssessment::load(
        args.compatibility_cache.as_deref(),
        &args.compatibility_keys,
        unix_seconds(),
    );
    let runtime = build_runtime(&data_dir, &observed_capabilities, compatibility)?;
    serve_local(runtime, &args, &data_dir).await
}

fn build_runtime(
    data_dir: &std::path::Path,
    observed_capabilities: &CapabilitySet,
    compatibility: CompatibilityAssessment,
) -> Result<RuntimeFacade, Box<dyn std::error::Error>> {
    let capabilities = validate_observed_capabilities(observed_capabilities)?;
    let ledger = SqliteLedger::open(data_dir.join("gent.db"))?;
    let attachments = AttachmentService::new(
        ledger.clone(),
        FileAttachmentBlobs::open(data_dir.join("attachments"))?,
    );
    let coordinator = Coordinator::new(ledger.clone(), capabilities);
    coordinator.persist_capability_catalog()?;
    Ok(RuntimeFacade {
        public_runs: observer_service(coordinator.clone(), compatibility.clone()),
        attachments,
        coordinator,
        dependencies: DependencyCatalog::with_compatibility(compatibility),
        dependency_actions: DependencyActionService::new(
            ledger,
            SystemDependencyExecutor::new(SystemDependencyInstaller),
        ),
    })
}

#[cfg(unix)]
async fn serve_local(
    runtime: RuntimeFacade,
    args: &Args,
    data_dir: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let socket = args
        .socket
        .clone()
        .unwrap_or_else(|| data_dir.join("gentd.sock"));
    transport::serve(UnixListener::bind(socket)?, runtime).await
}

#[cfg(windows)]
async fn serve_local(
    runtime: RuntimeFacade,
    _: &Args,
    data_dir: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    transport_windows::serve_named_pipe(&pipe_name(data_dir), runtime).await
}

#[cfg(windows)]
fn pipe_name(data_dir: &std::path::Path) -> String {
    format!(r"\\.\pipe\gentd-{:016x}", endpoint_hash(data_dir))
}

#[cfg(windows)]
fn endpoint_hash(data_dir: &std::path::Path) -> u64 {
    data_dir
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

#[derive(Clone, Debug)]
struct RuntimeFacade {
    attachments: AttachmentService<SqliteLedger, FileAttachmentBlobs>,
    coordinator: Coordinator<SqliteLedger>,
    dependencies: DependencyCatalog,
    dependency_actions:
        DependencyActionService<SqliteLedger, SystemDependencyExecutor<SystemDependencyInstaller>>,
    public_runs: DaemonPublicRuns,
}

impl api::RuntimeApi for RuntimeFacade {
    fn capabilities(&self) -> Result<CapabilitySet, String> {
        self.coordinator
            .status()
            .map(|status| status.capabilities)
            .map_err(|error| error.to_string())
    }

    fn status(&self) -> Result<HostStatus, String> {
        self.coordinator.status().map_err(|error| error.to_string())
    }
    fn submit(&self, command: Command) -> Result<Receipt, String> {
        self.coordinator
            .submit(&command)
            .map_err(|error| error.to_string())
    }
    fn resume_events(&self, cursor: u64) -> Result<EventResume, String> {
        self.coordinator
            .resume_events(cursor)
            .map_err(|error| error.to_string())
    }
    fn doctor(&self) -> DoctorReport {
        self.dependencies.doctor()
    }
    fn dependency_plan(&self, request: DependencyPlanRequest) -> DependencyPlan {
        self.dependencies.plan(request)
    }
    fn dependency_action(
        &self,
        request: DependencyActionRequest,
    ) -> Result<DependencyActionResult, String> {
        let plan = self.dependencies.plan(DependencyPlanRequest {
            provider: request.provider,
            action: request.action,
        });
        self.dependency_actions
            .execute(&request, &plan)
            .map_err(|error| error.to_string())
    }
    fn attachment(&self, frame: AttachmentFrame) -> Result<AttachmentFrame, String> {
        attachment_api::handle(&self.attachments, frame)
    }
    fn submit_decision(&self, command: DecisionCommand) -> Result<DecisionSubmission, String> {
        self.coordinator
            .submit_decision(command)
            .map(decision_submission)
            .map_err(|error| error.to_string())
    }
    fn apply_decision_evidence(
        &self,
        decision_id: String,
        evidence: DecisionEvidence,
    ) -> Result<DecisionSettlement, String> {
        self.coordinator
            .apply_decision_evidence(&decision_id, decision_evidence(evidence))
            .map_err(|error| error.to_string())
    }
    fn start_public_run(
        &self,
        request: gent_protocol::PublicRunStartRequest,
    ) -> Result<gent_protocol::PublicRunResponse, String> {
        self.public_runs
            .start(request)
            .map_err(|error| error.to_string())
    }
    fn resume_public_run(
        &self,
        request: gent_protocol::PublicRunResumeRequest,
    ) -> Result<gent_protocol::PublicRunResponse, String> {
        self.public_runs
            .resume(request)
            .map_err(|error| error.to_string())
    }
    fn interrupt_public_run(
        &self,
        request: gent_protocol::PublicRunInterruptRequest,
    ) -> Result<gent_protocol::PublicRunResponse, String> {
        self.public_runs
            .interrupt(request)
            .map_err(|error| error.to_string())
    }
    fn conversation_status(&self, conversation_id: &str) -> Result<ConversationStatus, String> {
        self.coordinator
            .conversation_status(conversation_id)
            .map_err(|error| error.to_string())
    }
    fn conversation_timeline(&self, conversation_id: &str) -> Result<ConversationTimeline, String> {
        self.coordinator
            .conversation_timeline(conversation_id)
            .map_err(|error| error.to_string())
    }
}

fn decision_submission(outcome: DecisionCommandOutcome) -> DecisionSubmission {
    match outcome {
        DecisionCommandOutcome::Accepted(decision) => DecisionSubmission::Accepted(decision),
        DecisionCommandOutcome::Duplicate(decision) => DecisionSubmission::Duplicate(decision),
        DecisionCommandOutcome::IdempotencyConflict {
            existing_decision_id,
        } => DecisionSubmission::IdempotencyConflict {
            existing_decision_id,
        },
        DecisionCommandOutcome::DecisionIdConflict {
            existing_idempotency_key,
        } => DecisionSubmission::DecisionIdConflict {
            existing_idempotency_key,
        },
    }
}

const fn decision_evidence(evidence: DecisionEvidence) -> CoreDecisionEvidence {
    match evidence {
        DecisionEvidence::ProviderAcknowledged => CoreDecisionEvidence::ProviderAcknowledged,
        DecisionEvidence::ProviderSettled => CoreDecisionEvidence::ProviderSettled,
        DecisionEvidence::AcknowledgementUnprovable => {
            CoreDecisionEvidence::AcknowledgementUnprovable
        }
        DecisionEvidence::RecoveryRequired => CoreDecisionEvidence::RecoveryRequired,
    }
}

fn default_data_dir() -> PathBuf {
    directories::ProjectDirs::from("ar", "Gent", "Gent").map_or_else(
        || PathBuf::from(".gent"),
        |directories| directories.data_local_dir().to_path_buf(),
    )
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
