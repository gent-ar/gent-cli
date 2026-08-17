// Generic command-frame mapping kept separate from IPC connection routing.

pub(crate) fn command_frame<R: RuntimeApi>(
    runtime: &R,
    raw: Value,
    client_supports_resync: bool,
) -> Result<WireFrame, String> {
    match serde_json::from_value::<WireFrame>(raw) {
        Ok(WireFrame::OnboardingRequest) => Ok(WireFrame::Onboarding(runtime.onboarding())),
        Ok(WireFrame::StatusRequest) => runtime.status().map(WireFrame::Status),
        Ok(WireFrame::DoctorRequest) => Ok(WireFrame::DoctorReport(runtime.doctor())),
        Ok(WireFrame::DependencyPlanRequest(request)) => {
            Ok(WireFrame::DependencyPlan(runtime.dependency_plan(request)))
        }
        Ok(WireFrame::DependencyActionRequest(request)) => runtime
            .dependency_action(request)
            .map(WireFrame::DependencyActionResult),
        Ok(WireFrame::DecisionSubmit(command)) => runtime
            .submit_decision(command)
            .map(WireFrame::DecisionSubmission),
        Ok(WireFrame::DecisionRecovery {
            decision_id,
            evidence,
        }) => runtime
            .apply_decision_recovery(decision_id, evidence)
            .map(WireFrame::DecisionSettlement),
        Ok(WireFrame::DecisionEvidence { .. }) => {
            Err("provider decision evidence is restricted to daemon-owned lifecycle inputs".into())
        }
        Ok(WireFrame::PublicRunStart(request)) => runtime
            .start_public_run(request)
            .map(WireFrame::PublicRunResponse),
        Ok(WireFrame::PublicRunResume(request)) => runtime
            .resume_public_run(request)
            .map(WireFrame::PublicRunResponse),
        Ok(WireFrame::PublicRunInterrupt(request)) => runtime
            .interrupt_public_run(request)
            .map(WireFrame::PublicRunResponse),
        Ok(WireFrame::Command(command)) => runtime.submit(command).map(WireFrame::Receipt),
        Ok(WireFrame::Subscribe { after_cursor }) => runtime
            .resume_events(after_cursor)
            .and_then(|resume| event_frame(resume, client_supports_resync)),
        Ok(_) | Err(_) => Err("frame is not valid after negotiation".into()),
    }
}

pub(crate) fn event_frame(
    resume: EventResume,
    client_supports_resync: bool,
) -> Result<WireFrame, String> {
    match resume {
        EventResume::Delta { events } => Ok(WireFrame::Events { events }),
        EventResume::Resync { snapshot, events } if client_supports_resync => {
            Ok(WireFrame::EventResync { snapshot, events })
        }
        EventResume::Resync { .. } => Err("event resync requires an upgraded client".into()),
    }
}
