# Scoped six-capability native cutover plan

## Relationship to the other plan docs in this folder

The other docs here (`gentd-source-of-truth-contract.md`, `native-agent-chat-cutover-map.md`,
`implementation-backlog.md`, `native-surface-disposition.md`) describe a larger rewrite: a generic
"catalog record" system for every UI vocabulary (provider/model/mode/permission-policy), and a
single `agent-chat-projection-v1` capability that replaces six existing capabilities with one
snapshot+delta contract. None of that exists yet, and native cutover under those docs is explicitly
gated behind it (`README.md`, "Gate before native integration starts").

This plan does not depend on any of that. It covers landing exactly the six capabilities built in
gent-cli commit `dbe6b44` ("Build remaining Gentd agent-chat capabilities and consolidate
outstanding work") — system-prompt/config, resume, fork, checkpoint, side-questions, git/worktree
status — as **direct typed calls through the existing `GentdIpcClient`**, keeping the current Dart
data-model shapes wherever they still fit. It can land in full before, after, or independent of the
catalog/projection rework; nothing here blocks or is blocked by it. Do not merge scope between the
two plans — a reader who wants the generic-catalog rewrite should keep reading the other docs, not
this one.

Path convention: `crates/…` is this repo; unqualified `lib/…` is
`/Users/ivanmatiasfort/Clouseau/clouseau-app/app/lib/`, relative to its root.

## Shared foundation already in place

`lib/service/gentd/gentd_ipc_client.dart` already implements everything a new capability needs:
capability-negotiated `_open`/`_request` helpers, a `requiredCapability` check against the
handshake (`gentdConversationIndexCapability` etc. — one `const` per capability, checked in
`_open`), typed `fromJson`/`toJson` result classes, and `GentdProtocolException` as the uniform
error type. Every capability below follows that exact pattern: add one capability-string `const`,
one or more typed result classes, and one or more methods on `GentdIpcClient`. No new transport
code, no new negotiation code.

**Local vs. remote.** The native app's Mac/PC/Linux instance already re-serves parts of its own
state to paired remote devices over its embedded HTTP server (see
`lib/provider/network/server/controller/agent_chat_controller.dart` for the host-side handler shape
and `lib/provider/remote_agent_data_provider.dart` for how a remote device consumes it). Per the
Remote Parity Rule, none of the six capabilities below gets a second implementation for remote: the
Mac's embedded server adds one new REST route per capability that internally calls the exact same
`GentdIpcClient` method the local UI calls directly, and the remote-facing provider calls that
route instead of `GentdIpcClient` directly. Getting the local path right is what makes the remote
path correct — there is no separate remote business logic to write, only a thin HTTP passthrough
on the host side and an HTTP client call on the remote side.

**Unsupported capability.** If the connected gentd's handshake doesn't advertise a capability (older
daemon, mid-rollout), `GentdIpcClient._open` already throws `GentdProtocolException`. Each surface
below must catch that specifically at the UI boundary and degrade to hiding the control (not
crashing, not silently no-opping) — e.g. the checkpoint restore option isn't shown at all rather
than shown and failing on tap. This mirrors how missing capabilities should already be handled
elsewhere; do not invent a new error-surfacing convention per capability.

## Build order

Smallest / lowest-risk first, so each capability is independently shippable and testable before the
next starts — the same discipline the Rust build used.

1. **Git/worktree status** — new capability, but purely read-only, zero side effects, easiest to
   validate end-to-end.
2. **Resume cleanup** — no new capability at all; this is a deletion pass once the other cutover
   work has proven the pattern.
3. **System prompt / advanced config** — one small revision-guarded record.
4. **Fork** — extends the already-negotiated `agent-chat-intents-v1`, no new capability string.
5. **Checkpoint** — dedicated capability, and the first one where a native action (restore) mutates
   workspace files directly on the daemon side.
6. **Side questions** — the most complex: replaces an entire local subsystem (helper-process spawn,
   bounding, concurrency caps) with a non-streaming ask/poll/push model.

---

## 1. Git/worktree status — `workspace-git-v1`

**Frame** (`crates/gent-protocol/src/workspace_git.rs`): `StatusRequest{workspace_id}` →
`Status{workspace_id, report: Option<WorkspaceGitReport>}` (`None` = not a git repo);
`SubReposRequest{workspace_id}` → `SubRepos{workspace_id, canonical_paths}`.
`WorkspaceGitReport{repository_root, branch, files: Vec<WorkspaceGitFileStatus>, worktrees}`;
`WorkspaceGitFileStatus{index_status, worktree_status, path, original_path}` (rename-aware);
`WorkspaceGitWorktree{canonical_path, branch, head, is_detached, is_locked}` — real
`git worktree list` data, richer than the current native model (no worktree list today).

**Dart work**: add `gentdWorkspaceGitCapability = 'workspace-git-v1'` and `GentdIpcClient.workspaceGitStatus(workspaceId)` /
`.workspaceGitSubRepos(workspaceId)`, with `GentdWorkspaceGitReport`/`GentdWorkspaceGitFileStatus`/
`GentdWorkspaceGitWorktree` mirroring the Rust shapes exactly (field-for-field, matching the
existing `fromJson` validation style in the file). Thin `lib/service/local_git_service.dart`'s
*read* methods (`getInfo`, `getHeaderInfo`, `checkRemoteStatus`, `branches`, `stashList`,
`repoRoot`) down to calls into the new client methods; delete `lib/model/git_info.dart`'s parsing
factories (`fromPorcelain`/`fromOneline`/`fromLogFormat`/`fromLine`) since parsing now happens once
server-side — keep `GitInfo` itself as a thin `fromJson` view over the wire shape.
**Do not touch** `local_git_service.dart`'s *mutating* methods (stage/commit/push/pull/branch/stash
create) — those stay native-only; gentd's capability is read-only by design (matching
`crates/gent-protocol/src/workspace_git.rs`'s own doc comment: "no mutation... is part of this
protocol").

**Local/remote**: `lib/provider/repo_cache_provider.dart` keeps its refcounted `RepoKey` cache
structure — that's orthogonal to where the fetch executes — but `createGitService`'s local-vs-remote
branch and `_findSubReposLocal` get replaced by the capability call (local) / new REST route
(remote).

**Push**: out of scope for this plan (matches the Rust side — `workspace-git-v1` is poll-only
today; a future fs-watch-backed push notification is a separate, not-yet-built piece on both sides).
Poll on the same cadence the native UI already uses for git status today.

**Acceptance**: opening a workspace's repo panel shows the same branch/file-status/worktree data
whether the workspace is local or on a paired remote device; a non-git workspace shows the existing
"not a repository" UI rather than an error.

## 2. Resume cleanup — no new capability

Resume is already fully implemented server-side and provider-neutral (existing `SendPrompt` intent
on `agent-chat-intents-v1` — gentd decides resume-vs-fresh internally via
`PublicRunService::start_or_resume`, already used by every conversation, not new this plan).

**Dart work**: delete the client-side resumability table and branch logic — grep
`resumableSessions`/`ResumableSessions` and any provider/runtime capability table in
`lib/service/agent_chat/agent_chat_spawn.dart` (or wherever it now lives — verify the exact
location before deleting; do not delete by pattern-matching alone) and the fast-fail auto-retry path
alongside it. After deletion, native only ever sends `SendPrompt`; it never chooses resume vs. fresh
itself. There is no new frame to add — this step is pure deletion once the surrounding capabilities
(especially fork and checkpoint, which also touch conversation/run selection) are cut over, so
there's no half-migrated state where some paths still branch on a client-side resumability guess.

**Acceptance**: turn 2 of an existing conversation resumes correctly with the client-side table
fully removed; killing gentd mid-session and reconnecting still resumes on the next prompt (gentd's
`start_or_resume`, not a client guess, decides).

## 3. System prompt / advanced config — `agent-chat-conversation-config-v1`

**Frame** (`crates/gent-protocol/src/agent_chat_conversation_config.rs`):
`Current{conversation_id}` → `CurrentConfig{config: Option<Record>, unsupported_for_provider}`;
`Save{config}` → `Saved{config, unsupported_for_provider}`.
`AgentChatConversationConfigRecord{conversation_id, revision, system_prompt: Option<String>,
append_system_prompt: bool, max_turns: Option<u32>, disallowed_tools: Vec<String>}`.
`unsupported_for_provider: Vec<AgentChatConversationConfigUnsupportedField>` —
`SystemPromptOverride | MaxTurns | DisallowedTools` — populated for Codex/Claurst when the
conversation's current provider can't honor a non-append override, a turn cap, or a tool denylist;
Claude has none. `Save` is revision-guarded (optimistic concurrency) — a stale `revision` is
rejected, matching the existing `permission_policy.rs`-style pattern already used elsewhere in this
protocol.

**Dart work**: add the capability const, `GentdConversationConfigRecord` (field-for-field),
`GentdIpcClient.currentConversationConfig(conversationId)` / `.saveConversationConfig(record)`.
Delete the editing UI in `lib/widget/agent_chat/panel_models.dart`'s local-state version, the
manifest `ArgRule` ↔ `systemPrompt`/`appendSystemPrompt` mapping in
`lib/model/agent_adapter.dart`'s `generic_adapter` path and its spawn-time call site, and
`lib/service/agent_chat/model_settings_controller.dart`'s local setter — replace with a load-on-open
`currentConversationConfig` call and a `saveConversationConfig` call on submit, surfacing
`unsupported_for_provider` as disabled/annotated fields rather than fields that silently do nothing
(this fixes the confirmed native bug where Codex silently dropped `systemPrompt` — call this out in
the PR description when this lands, since it's a real behavior change).

**Local/remote**: same host-serves-a-REST-route pattern as capability 1.

**Acceptance**: setting a system prompt on a Claude conversation changes its next turn's launched
arguments; the same field on a Codex conversation shows as unsupported in the UI rather than being
silently accepted and ignored; a stale-revision save (e.g. two devices editing at once) is rejected
and the UI reloads the current record rather than clobbering it.

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

**Local/remote**: same pattern.

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

**Local/remote**: same pattern. Restore is the first mutating action in this plan that changes
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

**Local/remote**: same REST-route pattern for ask/cancel/list; the push-event path needs the Mac's
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
