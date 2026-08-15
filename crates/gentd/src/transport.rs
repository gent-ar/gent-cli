//! Local IPC adapter. It only knows the `RuntimeApi` port, never persistence or providers.

use gent_protocol::{
    DecisionEvidence, DecisionSubmission, DependencyActionRequest, DependencyActionResult,
    DependencyPlan, DependencyPlanRequest, PublicRunInterruptRequest, PublicRunResponse,
    PublicRunResumeRequest, PublicRunStartRequest, WireFrame, negotiate, read_frame, write_frame,
};
use gent_runtime::catalog::{RuntimeCapability, capability_set};
use gent_types::{
    CapabilitySet, Command, DecisionCommand, DecisionSettlement, DoctorReport, Event, HostStatus,
    PROTOCOL_MAX, PROTOCOL_MIN, Receipt,
};
use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(unix)]
use tokio::net::UnixListener;

pub trait RuntimeApi: Clone + Send + Sync + 'static {
    fn capabilities(&self) -> Result<CapabilitySet, String>;
    fn status(&self) -> Result<HostStatus, String>;
    fn submit(&self, command: Command) -> Result<Receipt, String>;
    fn events_after(&self, cursor: u64) -> Result<Vec<Event>, String>;
    fn doctor(&self) -> DoctorReport;
    fn dependency_plan(&self, request: DependencyPlanRequest) -> DependencyPlan;
    fn dependency_action(&self, request: DependencyActionRequest) -> DependencyActionResult;
    fn submit_decision(&self, command: DecisionCommand) -> Result<DecisionSubmission, String>;
    fn apply_decision_evidence(
        &self,
        decision_id: String,
        evidence: DecisionEvidence,
    ) -> Result<DecisionSettlement, String>;
    fn start_public_run(&self, request: PublicRunStartRequest)
    -> Result<PublicRunResponse, String>;
    fn resume_public_run(
        &self,
        request: PublicRunResumeRequest,
    ) -> Result<PublicRunResponse, String>;
    fn interrupt_public_run(
        &self,
        request: PublicRunInterruptRequest,
    ) -> Result<PublicRunResponse, String>;
}

/// Reports the capabilities backed by concrete post-handshake handlers in this adapter.
#[must_use]
pub(crate) fn observed_capabilities() -> CapabilitySet {
    capability_set([
        RuntimeCapability::Decisions,
        RuntimeCapability::Events,
        RuntimeCapability::HostEpoch,
        RuntimeCapability::Receipts,
    ])
}

#[cfg(unix)]
pub async fn serve<R: RuntimeApi>(
    listener: UnixListener,
    runtime: R,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        let (stream, _) = listener.accept().await?;
        let runtime = runtime.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_connection(stream, runtime).await {
                eprintln!("gentd connection closed: {error}");
            }
        });
    }
}

pub(crate) async fn serve_connection<S, R>(
    mut stream: S,
    runtime: R,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: RuntimeApi,
{
    let WireFrame::Hello(hello) = read_frame(&mut stream).await? else {
        return write_error(
            &mut stream,
            "handshakeRequired",
            "hello must be the first frame",
        )
        .await;
    };
    let capabilities = match runtime.capabilities() {
        Ok(capabilities) => capabilities,
        Err(message) => return write_error(&mut stream, "capabilityUnavailable", &message).await,
    };
    match negotiate(&hello, PROTOCOL_MIN, PROTOCOL_MAX, &capabilities) {
        Ok(answer) => write_frame(&mut stream, &WireFrame::Negotiated(answer)).await?,
        Err(error) => return write_error(&mut stream, "upgradeRequired", &error.to_string()).await,
    }
    loop {
        let frame = match read_frame(&mut stream).await? {
            WireFrame::StatusRequest => runtime.status().map(WireFrame::Status),
            WireFrame::DoctorRequest => Ok(WireFrame::DoctorReport(runtime.doctor())),
            WireFrame::DependencyPlanRequest(request) => {
                Ok(WireFrame::DependencyPlan(runtime.dependency_plan(request)))
            }
            WireFrame::DependencyActionRequest(request) => Ok(WireFrame::DependencyActionResult(
                runtime.dependency_action(request),
            )),
            WireFrame::DecisionSubmit(command) => runtime
                .submit_decision(command)
                .map(WireFrame::DecisionSubmission),
            WireFrame::DecisionEvidence {
                decision_id,
                evidence,
            } => runtime
                .apply_decision_evidence(decision_id, evidence)
                .map(WireFrame::DecisionSettlement),
            WireFrame::PublicRunStart(request) => runtime
                .start_public_run(request)
                .map(WireFrame::PublicRunResponse),
            WireFrame::PublicRunResume(request) => runtime
                .resume_public_run(request)
                .map(WireFrame::PublicRunResponse),
            WireFrame::PublicRunInterrupt(request) => runtime
                .interrupt_public_run(request)
                .map(WireFrame::PublicRunResponse),
            WireFrame::Command(command) => runtime.submit(command).map(WireFrame::Receipt),
            WireFrame::Subscribe { after_cursor } => runtime
                .events_after(after_cursor)
                .map(|events| WireFrame::Events { events }),
            _ => Err("frame is not valid after negotiation".into()),
        };
        match frame {
            Ok(frame) => write_frame(&mut stream, &frame).await?,
            Err(message) => write_error(&mut stream, "invalidCommand", &message).await?,
        }
    }
}

async fn write_error<S>(
    stream: &mut S,
    code: &str,
    message: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
{
    write_frame(
        stream,
        &WireFrame::Error {
            code: code.into(),
            message: message.into(),
        },
    )
    .await?;
    Ok(())
}
