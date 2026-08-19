//! Pure, read-only comparison of legacy lifecycle observations with Gent's projection rules.

use gent_types::{
    ConversationLiveStatus, LegacyLifecycleTap, ObserverDiagnostic, ObserverDiagnosticCode,
};

use crate::{LifecycleProjection, project_lifecycle_signal, projected_live_status};

/// Ephemeral shadow state for one independently observed legacy lifecycle stream.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ObserverProjection {
    lifecycle: LifecycleProjection,
}

/// The result of a single comparison. Diagnostics never mutate external state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObserverComparison {
    pub projection: ObserverProjection,
    pub diagnostic: Option<ObserverDiagnostic>,
}

impl ObserverProjection {
    /// Projects one contiguous legacy lifecycle observation and compares its reported status.
    #[must_use]
    pub fn compare(self, tap: &LegacyLifecycleTap) -> ObserverComparison {
        compare_legacy_tap(self, tap)
    }

    /// Returns the latest independent Gent projection, without any legacy content.
    #[must_use]
    pub fn status(&self) -> ConversationLiveStatus {
        projected_live_status(&self.lifecycle)
    }
}

/// Projects a content-safe legacy fact using Gent's pure lifecycle reducer.
#[must_use]
pub fn compare_legacy_tap(
    state: ObserverProjection,
    tap: &LegacyLifecycleTap,
) -> ObserverComparison {
    let expected_cursor = state
        .lifecycle
        .last_cursor
        .map_or(1, |cursor| cursor.saturating_add(1));
    if tap.cursor < expected_cursor {
        return result(
            state,
            Some(diagnostic(ObserverDiagnosticCode::Duplicate, tap, None)),
        );
    }
    if tap.cursor > expected_cursor {
        return result(
            state,
            Some(diagnostic(ObserverDiagnosticCode::CursorGap, tap, None)),
        );
    }
    let lifecycle = project_lifecycle_signal(state.lifecycle, tap.cursor, &tap.signal).state;
    let projection = ObserverProjection { lifecycle };
    let expected = projection.status();
    let issue = (expected != tap.reported)
        .then(|| diagnostic(ObserverDiagnosticCode::StatusMismatch, tap, Some(expected)));
    result(projection, issue)
}

fn result(
    projection: ObserverProjection,
    diagnostic: Option<ObserverDiagnostic>,
) -> ObserverComparison {
    ObserverComparison {
        projection,
        diagnostic,
    }
}

fn diagnostic(
    code: ObserverDiagnosticCode,
    tap: &LegacyLifecycleTap,
    expected: Option<ConversationLiveStatus>,
) -> ObserverDiagnostic {
    ObserverDiagnostic {
        code,
        cursor: tap.cursor,
        event_id: tap.event_id.clone(),
        receipt_id: tap.receipt_id.clone(),
        expected,
        reported: (code == ObserverDiagnosticCode::StatusMismatch).then(|| tap.reported.clone()),
    }
}

#[cfg(test)]
mod tests {
    use gent_types::{
        ConversationLiveStatus, LegacyLifecycleTap, NormalizedLifecycleSignal,
        ObserverDiagnosticCode, ReceiptId, TurnPhase, WorkPhase,
    };

    use super::ObserverProjection;

    fn tap(cursor: u64, signal: NormalizedLifecycleSignal) -> LegacyLifecycleTap {
        LegacyLifecycleTap {
            cursor,
            event_id: format!("event-{cursor}"),
            receipt_id: ReceiptId(format!("receipt-{cursor}")),
            signal,
            reported: ConversationLiveStatus {
                cursor,
                ..ConversationLiveStatus::default()
            },
        }
    }

    #[test]
    fn projection_matches_waiting_subagent_command_and_attention_states() {
        let mut state = ObserverProjection::default();
        let mut waiting = tap(
            1,
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::WaitingQuestion,
            },
        );
        waiting.reported.is_processing = true;
        state = state.compare(&waiting).projection;
        let mut child = tap(
            2,
            NormalizedLifecycleSignal::ChildPhase {
                child_id: "child".into(),
                phase: WorkPhase::Running,
            },
        );
        child.reported.is_waiting_for_subagents = true;
        child.reported.has_live_subagent_work = true;
        state = state.compare(&child).projection;
        let mut command = tap(
            3,
            NormalizedLifecycleSignal::CommandPhase {
                command_id: "command".into(),
                phase: WorkPhase::WaitingPermission,
            },
        );
        command.reported.is_waiting_for_subagents = true;
        command.reported.has_live_subagent_work = true;
        command.reported.is_waiting_for_command = true;
        command.reported.has_live_command_work = true;
        state = state.compare(&command).projection;
        let mut attention = tap(4, NormalizedLifecycleSignal::AttentionRequired);
        attention.reported = state.status();
        attention.reported.cursor = 4;
        attention.reported.needs_attention = true;
        assert!(state.compare(&attention).diagnostic.is_none());
    }

    #[test]
    fn duplicate_gap_and_status_mismatch_are_deterministic() {
        let state = ObserverProjection::default();
        let first = tap(1, NormalizedLifecycleSignal::AttentionCleared);
        let state = state.compare(&first).projection;
        assert_eq!(
            state.clone().compare(&first).diagnostic.unwrap().code,
            ObserverDiagnosticCode::Duplicate
        );
        assert_eq!(
            state
                .clone()
                .compare(&tap(3, NormalizedLifecycleSignal::AttentionCleared))
                .diagnostic
                .unwrap()
                .code,
            ObserverDiagnosticCode::CursorGap
        );
        let mismatch = state.compare(&tap(2, NormalizedLifecycleSignal::AttentionRequired));
        assert_eq!(
            mismatch.diagnostic.unwrap().code,
            ObserverDiagnosticCode::StatusMismatch
        );
    }
}
