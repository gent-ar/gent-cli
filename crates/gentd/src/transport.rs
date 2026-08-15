//! Local IPC adapter. It only knows the `RuntimeApi` port, never persistence or providers.

use gent_protocol::{
    DependencyActionRequest, DependencyActionResult, DependencyPlan, DependencyPlanRequest,
    WireFrame, negotiate, read_frame, write_frame,
};
use gent_types::{
    CapabilitySet, Command, DoctorReport, Event, HostStatus, PROTOCOL_MAX, PROTOCOL_MIN, Receipt,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixListener;

const CAPABILITIES: &[&str] = &["events", "host-epoch", "receipts"];

pub trait RuntimeApi: Clone + Send + Sync + 'static {
    fn status(&self) -> Result<HostStatus, String>;
    fn submit(&self, command: Command) -> Result<Receipt, String>;
    fn events_after(&self, cursor: u64) -> Result<Vec<Event>, String>;
    fn doctor(&self) -> DoctorReport;
    fn dependency_plan(&self, request: DependencyPlanRequest) -> DependencyPlan;
    fn dependency_action(&self, request: DependencyActionRequest) -> DependencyActionResult;
}

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

async fn serve_connection<S, R>(
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
    let capabilities = CapabilitySet(CAPABILITIES.iter().map(ToString::to_string).collect());
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

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use gent_protocol::{
        DependencyAction, DependencyActionRequest, DependencyActionState, DependencyPlan,
        DependencyPlanRequest, DependencyProvider, Hello, WireFrame, read_frame, write_frame,
    };
    use gent_types::{
        CapabilitySet, DoctorReport, HostEpoch, HostStatus, PROTOCOL_MAX, PROTOCOL_MIN,
    };
    use tokio::io::duplex;

    use super::{RuntimeApi, serve_connection};

    #[derive(Clone, Debug)]
    struct FakeRuntime;

    impl RuntimeApi for FakeRuntime {
        fn status(&self) -> Result<HostStatus, String> {
            Ok(HostStatus {
                host_epoch: HostEpoch(1),
                protocol_min: PROTOCOL_MIN,
                protocol_max: PROTOCOL_MAX,
                capabilities: CapabilitySet::default(),
            })
        }

        fn submit(&self, _: gent_types::Command) -> Result<gent_types::Receipt, String> {
            Err("not used".into())
        }

        fn events_after(&self, _: u64) -> Result<Vec<gent_types::Event>, String> {
            Ok(Vec::new())
        }

        fn doctor(&self) -> DoctorReport {
            DoctorReport {
                dependencies: Vec::new(),
            }
        }

        fn dependency_plan(&self, request: DependencyPlanRequest) -> DependencyPlan {
            DependencyPlan {
                provider: request.provider,
                action: request.action,
                instruction: "review vendor installer".into(),
                consent_required: true,
            }
        }

        fn dependency_action(
            &self,
            request: DependencyActionRequest,
        ) -> gent_protocol::DependencyActionResult {
            gent_protocol::DependencyActionResult {
                plan: self.dependency_plan(DependencyPlanRequest {
                    provider: request.provider,
                    action: request.action,
                }),
                state: if request.consent_granted {
                    DependencyActionState::InstallerNotConfigured
                } else {
                    DependencyActionState::ConsentRequired
                },
            }
        }
    }

    fn hello() -> WireFrame {
        WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet::default(),
        })
    }

    #[tokio::test]
    async fn handshake_is_mandatory_before_requests() {
        let (mut client, server) = duplex(1024);
        let task = tokio::spawn(serve_connection(server, FakeRuntime));
        write_frame(&mut client, &WireFrame::StatusRequest)
            .await
            .unwrap();
        assert!(matches!(
            read_frame(&mut client).await.unwrap(),
            WireFrame::Error { code, .. } if code == "handshakeRequired"
        ));
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn typed_dependency_requests_need_consent_and_never_start_an_installer() {
        let (mut client, server) = duplex(1024);
        let task = tokio::spawn(serve_connection(server, FakeRuntime));
        write_frame(&mut client, &hello()).await.unwrap();
        assert!(matches!(
            read_frame(&mut client).await.unwrap(),
            WireFrame::Negotiated(_)
        ));
        write_frame(
            &mut client,
            &WireFrame::DependencyActionRequest(DependencyActionRequest {
                provider: DependencyProvider::Claude,
                action: DependencyAction::Install,
                consent_granted: false,
            }),
        )
        .await
        .unwrap();
        assert!(matches!(
            read_frame(&mut client).await.unwrap(),
            WireFrame::DependencyActionResult(result)
                if result.state == DependencyActionState::ConsentRequired
        ));
        drop(client);
        assert!(task.await.unwrap().is_err());
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
