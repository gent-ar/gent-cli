//! Observer-only composition of the public-driver lifecycle service.

use std::sync::Arc;

use gent_drivers::buffering::BufferPolicy;
use gent_drivers::interrupt::InterruptPolicy;
use gent_drivers::{DriverRunRunner, OutputLimits, SystemLauncher, SystemProcess};
use gent_runtime::{Coordinator, ProviderRunAuthority, PublicRunService};
use gent_store::SqliteLedger;

use crate::compatibility_assessment::CompatibilityAssessment;

const STREAM_CAPTURE_BYTES: usize = 64 * 1024;
const OUTPUT_FRAME_BYTES: usize = 16 * 1024;
const OUTPUT_TOTAL_BYTES: usize = 256 * 1024;
const BUFFERED_FRAMES: usize = 16;
const BUFFERED_BYTES: usize = 256 * 1024;
const INTERRUPT_GRACE_MS: u64 = 5_000;
const TERMINATE_GRACE_MS: u64 = 5_000;

/// The daemon's process-capable lifecycle service. The configured instance remains observer-only.
pub(crate) type DaemonPublicRuns = Arc<
    PublicRunService<
        SqliteLedger,
        DriverRunRunner<SystemLauncher, SystemProcess>,
        CompatibilityAssessment,
    >,
>;

/// Builds the shipped observer service without inspecting or starting an executable.
#[must_use]
pub(crate) fn observer_service(
    coordinator: Coordinator<SqliteLedger>,
    compatibility: CompatibilityAssessment,
) -> DaemonPublicRuns {
    let runner = DriverRunRunner::new(
        SystemLauncher::new(STREAM_CAPTURE_BYTES),
        OutputLimits::new(OUTPUT_FRAME_BYTES, OUTPUT_TOTAL_BYTES),
        BufferPolicy::new(BUFFERED_FRAMES, BUFFERED_BYTES, 0, 0)
            .expect("fixed daemon buffer policy is valid"),
        InterruptPolicy {
            interrupt_grace_ms: INTERRUPT_GRACE_MS,
            terminate_grace_ms: TERMINATE_GRACE_MS,
        },
    );
    Arc::new(PublicRunService::new(
        coordinator,
        runner,
        compatibility,
        ProviderRunAuthority::Observer,
    ))
}
