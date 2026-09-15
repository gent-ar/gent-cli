# Scoped six-capability native cutover plan — part 2

Continues `scoped-six-capability-cutover-plan.md` (relationship, shared foundation, testing approach,
build order, and capabilities 1–3); the same path convention applies.

## 4. Fork — `agent-chat-intents-v1` (existing capability, new variants)

**Frame** (`crates/gent-protocol/src/agent_chat_intent.rs`):
`ForkConversation{request_id, receipt_id, source_conversation_id, fork_through_message_id}` →
`Forked{request_id, receipt, source_conversation_id, conversation_id, run_id}`. Note this capability
is already negotiated by `GentdIpcClient._intent` (`gentdAgentChatIntentsCapability`) for
`createConversation`/`sendPrompt`/etc. — this is additive to an existing client method group, not a
new `_open` call.

**Dart work**: add `GentdIpcClient.forkConversation(sourceConversationId, forkThroughMessageId)`
returning a small `GentdForkedConversation{conversationId, runId}`. Delete
`forkConversation()`/related logic in `lib/util/agent_chat_actions/conversation_lifecycle.dart` (this
file exists and is the current fork implementation — verified this pass), the local fork-seed path,
and `lib/util/agent_chat_history_injection.dart` entirely (gentd's fresh-context rendering already
does this same job server-side — the native version becomes dead code, not a fallback to keep).

**Local/remote**: new route, same file and convention as the other conversation-lifecycle actions
already there (`new`, `rewind`, `rewind-restore`, `checkpoint-restore`):
`POST /api/mate/v1/agent-chat/:id/fork` in
`lib/provider/network/server/controller/agent_chat_routes_history.dart`
(`extension _AgentChatHistoryRoutes on AgentChatController`, registered via
`router.param(HttpMethod.post, ...)` inside `installHistoryRoutes`) — body carries
`forkThroughMessageId`, response carries the new `conversationId`/`runId`.

**Acceptance**: forking at message N in a conversation produces a new conversation whose first
prompt hits a fresh provider session seeded with exactly messages 1..N; forking a Codex-provider
conversation works identically to a Claude one (proves one canonical path, not a Claude-only
shortcut — this was explicitly verified server-side already).

## 5. Checkpoint — `agent-chat-checkpoint-v1`

**Frame** (`crates/gent-protocol/src/agent_chat_checkpoint.rs`):
`CaptureCheckpoint{conversation_id, run_id, message_ordinal, files: Vec<AgentChatFileSnapshot>}` →
`Captured{checkpoint}`; `ListCheckpoints{conversation_id}` → `Checkpoints{checkpoints}`;
`RestoreCheckpoint{conversation_id, checkpoint_id, restore_files: bool,
restore_files_confirmation: Option<String>}` → `Restored{conversation_id, checkpoint_id, run_id,
visible_through_ordinal, restored_files}`. `restore_files_confirmation` is required non-empty
whenever `restore_files=true`, enforced server-side (not client-trusted).
`AgentChatFileCheckpoint{checkpoint_id, conversation_id, run_id, message_ordinal,
created_at_unix_ms, files: Vec<AgentChatFileCheckpointFile>}`, and critically
`AgentChatFileCheckpointFile{file_path, storage_key, byte_len}` — **no file content**. This is a
real behavior difference from the current native model: gentd's restore **writes files to the
workspace directly, server-side**, from its own content-addressed blob store, and reports back only
which files it touched. The native `Checkpoint.fileSnapshots: Map<String, String>` (full content,
kept client-side) has no equivalent after cutover — content never crosses IPC in either direction.

**Dart work**: add the capability const, `GentdFileCheckpoint`/`GentdFileCheckpointFile` (metadata
only, no content field), `GentdIpcClient.captureCheckpoint(...)` / `.listCheckpoints(conversationId)`
/ `.restoreCheckpoint(...)`. Replace `Checkpoint` construction and `_preEditSnapshots` in
`lib/service/agent_chat/checkpoint_controller.dart` (verified this pass) with a call after each turn
that reads the changed files' content (native already knows this locally, pre-edit — it must still
read the bytes itself to send as `AgentChatFileSnapshot{file_path, content}` on capture; only the
*storage* becomes server-side) and calls `captureCheckpoint`. Update the model class to drop
`fileSnapshots`, keeping `modifiedFiles`. Keep `showAgentRestoreOptions` in
`lib/widget/agent_chat/panel_dialogs.dart` as pure UI (verified this pass, at the call site shown
below) — it already collects a confirmation string; thread that straight into
`restore_files_confirmation` instead of a Dart-local restore path:
```
showAgentRestoreOptions(context, checkpoint, colorScheme,
  onConfirm: ({required restoreCode}) => ref.notifier(agentChatProvider).restoreCheckpoint(...))
```
Bound capture size the same way native does today (2 MiB / file — verify the current constant
before removing it, since gentd enforces its own `MAX_CHECKPOINT_SNAPSHOT_BYTES` independently and
the two should not silently diverge) and retention count (`MAX_RETAINED_CHECKPOINTS`, currently 25
server-side).

**Local/remote**: `lib/provider/network/server/controller/agent_chat_routes_history.dart` **already
has** `POST /api/mate/v1/agent-chat/:id/checkpoint-restore` — today it restores from
`checkpoint['fileSnapshots']` stored client-side in `stateJson`. Keep the same route path and
request shape (`checkpointId`, `restoreCode` → `restoreFiles`), but replace the handler body with a
call to `GentdIpcClient.restoreCheckpoint(..., restoreFilesConfirmation: ...)`; the client-side
file-write loop and `persistence.getCheckpointSnapshots` lookup in the current handler become dead
code. `ListCheckpoints` needs a new route, e.g. `GET /api/mate/v1/agent-chat/:id/checkpoints`, same
file. `CaptureCheckpoint` likely needs **no remote route at all** — only the device actually running
the turn ever captures (a remote device only views and restores) — confirm this against how
`checkpoint_controller.dart`'s capture call site is reached before assuming it, but do not build a
remote capture route speculatively. Restore is the first mutating action in this plan that changes
workspace files on the *host* — for a remote device restoring a Mac's checkpoint, the REST route
must run on the Mac (where the files live), never attempt a remote filesystem write.

**Acceptance**: a turn editing 3 files produces one checkpoint with 3 file entries; restoring with
`restoreFiles: true` and no confirmation string is rejected before any file is touched (verify this
client-side too, not just server-side, so the UI never sends an invalid request); after restore, the
conversation's visible history stops at the checkpoint's ordinal until further prompts extend it
again.

## 6. Side questions — `agent-chat-side-question-v1`

**Frame** (`crates/gent-protocol/src/agent_chat_side_question.rs`):
`AskSideQuestion{conversation_id, question}` → `Asked{record}` (record status starts `Pending`);
`CancelSideQuestion{side_question_id}` → `Cancelled{record}`; `ListSideQuestions{conversation_id}`
→ `SideQuestions{side_questions}`. `AgentChatSideQuestionRecord{side_question_id, conversation_id,
question, status: Pending|Answered|Failed|Cancelled, answer: Option<String>,
failure_reason: Option<String>, created_at_unix_ms}`.

**This is non-streaming**, a deliberate v1 scope decision made during the Rust build — read the doc
comment at the top of `agent_chat_side_question.rs` before implementing the client. `Asked` returns
immediately with a `Pending` record. The final `Answered`/`Failed` record arrives one of two ways:
(a) poll `ListSideQuestions` again, or (b) subscribe to the existing `event-stream-v1` capability
(`GentdIpcClient.followLocalModelDownloads` is the existing pattern for consuming this stream —
mirror it, filtering on `event.kind == 'agentChatSideQuestionAnswered'` instead of
`'localModelDownload'`) and read the pushed `sideQuestionId`/`status` out of the event payload, then
re-fetch or trust the payload directly if it's sufficient. **Cancel is a durable-record-only
operation**: it marks the record `Cancelled` but does **not** kill the in-flight provider process
server-side (the runner traits don't expose a kill handle across that boundary — this is a known,
documented v1 limitation, not a bug to work around client-side). The native UI must reflect this
honestly: cancelling stops the UI from waiting/showing a spinner, but must not claim the underlying
work was interrupted.

Bounded-excerpt parity is already exact: gentd's `MAX_EXCERPT_MESSAGES = 8` /
`MAX_EXCERPT_BYTES = 12_000` (`crates/gent-runtime/src/agent_chat_side_question.rs`) match native's
existing `sideQuestionMessageLimit = 8` / `sideQuestionCharLimit = 12_000`
(`lib/util/side_question_context.dart`, verified this pass) exactly — this was intentional parity
during the Rust build, not a coincidence to reconcile.

**Dart work**: add the capability const, `GentdSideQuestionRecord`,
`GentdIpcClient.askSideQuestion(conversationId, question)` / `.cancelSideQuestion(sideQuestionId)` /
`.listSideQuestions(conversationId)`, plus a stream/poll consumer for the answered push event. This
is the one capability where the native cutover is a net *deletion* of an entire local subsystem:
delete the local helper-process spawn, the bounding/concurrency-cap Dart code, and
`side_question_context.dart`'s two functions (`recentSideQuestionMessages`,
`boundSideQuestionTranscript`) — they become dead code once bounding happens server-side; verify no
other caller depends on them before deleting. Keep the two named constants
(`sideQuestionMessageLimit`, `sideQuestionCharLimit`) only if a UI surface still needs to *display*
the bound (e.g. "showing last 8 messages") — otherwise delete the whole file.

**Local/remote**: three new routes, none of which exist today (verified — the only existing
side-question surface is `AgentChatController.registerSideQuestionProcessForTest`, the local
helper-process bookkeeping this plan deletes). Follow the file-per-concern convention the other
routes use (`agent_chat_routes_history.dart`, `_feed.dart`, `_messaging.dart`, `_connectors.dart`,
each `part of 'agent_chat_controller.dart'`, each an `extension on AgentChatController` with one
`installXRoutes(router)` entry point) and add a new
`lib/provider/network/server/controller/agent_chat_routes_side_question.dart` in the same shape,
with one new `installSideQuestionRoutes(router)` call added alongside the other `installXRoutes`
calls in `agent_chat_controller.dart`. Routes: `POST /api/mate/v1/agent-chat/:id/side-questions`
(ask), `POST /api/mate/v1/agent-chat/:id/side-questions/:sideQuestionId/cancel`,
`GET /api/mate/v1/agent-chat/:id/side-questions` (list). The push-event path needs the Mac's
existing mux/event relay to forward `agentChatSideQuestionAnswered` events to a subscribed remote
device the same way it already relays other event-stream events — do not build a second push
mechanism for remote.

**Acceptance**: asking a side question returns a `Pending` record immediately (UI shows
"thinking", not a blocking spinner that ties up the composer); the answer arrives via the push path
without polling, in under the same latency envelope the current local implementation has today;
asking a 4th concurrent question on one conversation is rejected client-visibly (server enforces
3/conversation, 8 total — the UI should not let a user queue past that silently and then show a
confusing rejection); cancelling a pending question stops the UI waiting on it without claiming the
provider call was interrupted.
