use super::{CodexRunnerEffect, CodexRunnerError, OwnedRun};
use crate::codex_turn::CodexTurnEffect;
use crate::supervisor::ProviderProcess;

pub(super) fn drain<P: ProviderProcess>(
    run: &mut OwnedRun<P>,
) -> Result<Option<Vec<CodexRunnerEffect>>, CodexRunnerError> {
    let mut effects = Vec::new();
    while run.output.queued_frames() > 0 {
        let (frame, _) = run.output.take_frame();
        let Some(frame) = frame else { break };
        if let Some(request_key) = super::control::cancelled_control_request_key(&frame) {
            run.controls.remove(&request_key);
        }
        let received = match run.turn.receive(&frame) {
            Err(crate::codex_turn::CodexTurnError::Session(
                crate::codex_session::CodexSessionError::ResumedThreadUnavailable,
            )) => {
                effects.push(CodexRunnerEffect::ResumeUnavailable);
                break;
            }
            received => received?,
        };
        for effect in received {
            match effect {
                CodexTurnEffect::Write(frame) => run.process.write_frame(&frame)?,
                CodexTurnEffect::Fact(fact) => effects.push(CodexRunnerEffect::Fact(fact)),
                CodexTurnEffect::Steer(outcome) => effects.push(CodexRunnerEffect::Steer(outcome)),
                CodexTurnEffect::ControlRequest(request) => {
                    run.controls
                        .insert(request.request_key.clone(), request.clone());
                    effects.push(CodexRunnerEffect::ControlRequest(request));
                }
            }
        }
    }
    effects.extend(
        crate::public_protocol::oversized_frames_skipped(run.output.take_skipped_frames())
            .map(CodexRunnerEffect::Fact),
    );
    Ok((!effects.is_empty()).then_some(effects))
}

pub(super) fn write<P: ProviderProcess>(
    process: &P,
    effect: CodexTurnEffect,
) -> Result<(), CodexRunnerError> {
    match effect {
        CodexTurnEffect::Write(frame) => process.write_frame(&frame)?,
        CodexTurnEffect::Fact(_)
        | CodexTurnEffect::ControlRequest(_)
        | CodexTurnEffect::Steer(_) => {}
    }
    Ok(())
}
