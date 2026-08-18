use super::{
    AgentChatControllerSnapshotBuilder, AgentChatControllerSnapshotRequest,
    AgentChatControllerSnapshotSource,
};
use crate::RuntimeError;
use gent_types::{
    AgentChatConversationDetail, AgentChatConversationSummary, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatSelection, ConversationStatus, HostEpoch, HostStatus,
    NormalizedTranscriptEvent, NormalizedTranscriptKind, NormalizedTranscriptPage,
};

struct Source {
    hosts: [HostStatus; 2],
    status: ConversationStatus,
    detail: AgentChatConversationDetail,
    page: NormalizedTranscriptPage,
    calls: std::cell::Cell<usize>,
    limit: std::cell::Cell<u16>,
}
impl AgentChatControllerSnapshotSource for Source {
    fn host_status(&self) -> Result<HostStatus, RuntimeError> {
        let call = self.calls.get();
        self.calls.set(call + 1);
        Ok(self.hosts[call].clone())
    }
    fn conversation_detail(&self, _: &str) -> Result<AgentChatConversationDetail, RuntimeError> {
        Ok(self.detail.clone())
    }
    fn transcript(
        &self,
        _: &str,
        _: Option<u64>,
        limit: u16,
    ) -> Result<NormalizedTranscriptPage, RuntimeError> {
        self.limit.set(limit);
        Ok(self.page.clone())
    }
    fn conversation_status(&self, _: &str) -> Result<ConversationStatus, RuntimeError> {
        Ok(self.status.clone())
    }
}
#[test]
fn combines_fenced_detail_page_and_status() {
    let source = source(HostEpoch(3), HostEpoch(3));
    let snapshot = AgentChatControllerSnapshotBuilder::read(&source, &request(true)).unwrap();
    assert_eq!(snapshot.host_epoch, HostEpoch(3));
    assert_eq!(snapshot.status, Some(status()));
    assert_eq!(source.limit.get(), 100);
}
#[test]
fn rejects_epoch_change() {
    assert!(
        AgentChatControllerSnapshotBuilder::read(
            &source(HostEpoch(3), HostEpoch(4)),
            &request(false)
        )
        .is_err()
    );
}
#[test]
fn rejects_selection_or_cursor_drift() {
    let mut selection_source = source(HostEpoch(3), HostEpoch(3));
    selection_source.detail.summary.selection.model = "other".into();
    assert!(AgentChatControllerSnapshotBuilder::read(&selection_source, &request(false)).is_err());
    let mut source = source(HostEpoch(3), HostEpoch(3));
    source.page.events.push(event(2));
    assert!(AgentChatControllerSnapshotBuilder::read(&source, &request(false)).is_err());
}
fn source(before: HostEpoch, after: HostEpoch) -> Source {
    Source {
        hosts: [host(before), host(after)],
        status: status(),
        detail: AgentChatConversationDetail {
            summary: summary(),
            runs: vec![],
        },
        page: NormalizedTranscriptPage {
            conversation_id: "conversation".into(),
            events: vec![event(2)],
            next_after_cursor: None,
        },
        calls: std::cell::Cell::new(0),
        limit: std::cell::Cell::new(0),
    }
}
fn request(include_status: bool) -> AgentChatControllerSnapshotRequest {
    AgentChatControllerSnapshotRequest {
        conversation_id: "conversation".into(),
        after_cursor: Some(1),
        transcript_limit: 500,
        expected_selection: Some(selection()),
        include_status,
    }
}
fn host(epoch: HostEpoch) -> HostStatus {
    HostStatus {
        host_epoch: epoch,
        protocol_min: 1,
        protocol_max: 1,
        capabilities: gent_types::CapabilitySet::default(),
    }
}
fn status() -> ConversationStatus {
    ConversationStatus {
        conversation_id: "conversation".into(),
        runs: vec![],
    }
}
fn summary() -> AgentChatConversationSummary {
    AgentChatConversationSummary {
        conversation_id: "conversation".into(),
        title: None,
        updated_at_unix_ms: 1,
        selection: selection(),
    }
}
fn selection() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "gpt".into(),
        effort: AgentChatEffort::Low,
        mode: AgentChatMode::Ask,
    }
}
fn event(cursor: u64) -> NormalizedTranscriptEvent {
    NormalizedTranscriptEvent {
        cursor,
        event_id: format!("event-{cursor}"),
        turn_id: "turn".into(),
        run_id: "run".into(),
        kind: NormalizedTranscriptKind::AssistantMessage,
        text: "hello".into(),
        is_partial: false,
    }
}
