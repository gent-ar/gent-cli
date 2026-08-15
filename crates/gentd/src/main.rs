mod dependency_catalog;
mod host_lock;
mod transport;
#[cfg(windows)]
mod transport_windows;
#[cfg(all(test, windows))]
mod transport_windows_tests;

use std::path::PathBuf;

use clap::Parser;
use gent_core::{DecisionCommandOutcome, DecisionEvidence as CoreDecisionEvidence};
use gent_protocol::{
    DecisionEvidence, DecisionSubmission, DependencyActionRequest, DependencyActionResult,
    DependencyPlan, DependencyPlanRequest,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    CapabilitySet, Command, DecisionCommand, DecisionSettlement, DoctorReport, Event, HostStatus,
    Receipt,
};
#[cfg(unix)]
use tokio::net::UnixListener;

use crate::dependency_catalog::DependencyCatalog;

const CAPABILITIES: &[&str] = &["decisions", "events", "host-epoch", "receipts"];

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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let data_dir = args.data_dir.clone().unwrap_or_else(default_data_dir);
    std::fs::create_dir_all(&data_dir)?;
    let _host_lock = host_lock::acquire(&data_dir)?;
    let runtime = RuntimeFacade {
        coordinator: Coordinator::new(
            SqliteLedger::open(data_dir.join("gent.db"))?,
            CapabilitySet(CAPABILITIES.iter().map(ToString::to_string).collect()),
        ),
        dependencies: DependencyCatalog,
    };
    serve_local(runtime, &args, &data_dir).await
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
    coordinator: Coordinator<SqliteLedger>,
    dependencies: DependencyCatalog,
}

impl transport::RuntimeApi for RuntimeFacade {
    fn status(&self) -> Result<HostStatus, String> {
        self.coordinator.status().map_err(|error| error.to_string())
    }
    fn submit(&self, command: Command) -> Result<Receipt, String> {
        self.coordinator
            .submit(&command)
            .map_err(|error| error.to_string())
    }
    fn events_after(&self, cursor: u64) -> Result<Vec<Event>, String> {
        self.coordinator
            .events_after(cursor)
            .map_err(|error| error.to_string())
    }
    fn doctor(&self) -> DoctorReport {
        self.dependencies.doctor()
    }
    fn dependency_plan(&self, request: DependencyPlanRequest) -> DependencyPlan {
        self.dependencies.plan(request)
    }
    fn dependency_action(&self, request: DependencyActionRequest) -> DependencyActionResult {
        self.dependencies.act(request)
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
