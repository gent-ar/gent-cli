mod dependency_catalog;
mod transport;

use std::path::PathBuf;

use clap::Parser;
use gent_protocol::{
    DependencyActionRequest, DependencyActionResult, DependencyPlan, DependencyPlanRequest,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, Command, DoctorReport, Event, HostStatus, Receipt};
use tokio::net::UnixListener;

use crate::dependency_catalog::DependencyCatalog;

const CAPABILITIES: &[&str] = &["events", "host-epoch", "receipts"];

#[derive(Debug, Parser)]
#[command(name = "gentd", about = "Gent's local runtime host")]
struct Args {
    /// Directory containing the socket and durable `SQLite` ledger.
    #[arg(long, env = "GENT_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Explicit Unix socket path, primarily for supervised launches and tests.
    #[arg(long)]
    socket: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let data_dir = args.data_dir.unwrap_or_else(default_data_dir);
    std::fs::create_dir_all(&data_dir)?;
    let socket = args.socket.unwrap_or_else(|| data_dir.join("gentd.sock"));
    let listener = UnixListener::bind(socket)?;
    let runtime = RuntimeFacade {
        coordinator: Coordinator::new(
            SqliteLedger::open(data_dir.join("gent.db"))?,
            CapabilitySet(CAPABILITIES.iter().map(ToString::to_string).collect()),
        ),
        dependencies: DependencyCatalog,
    };
    transport::serve(listener, runtime).await
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
}

fn default_data_dir() -> PathBuf {
    directories::ProjectDirs::from("ar", "Gent", "Gent").map_or_else(
        || PathBuf::from(".gent"),
        |directories| directories.data_local_dir().to_path_buf(),
    )
}
