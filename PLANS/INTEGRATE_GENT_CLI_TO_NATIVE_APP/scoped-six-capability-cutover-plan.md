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

## Testing approach

clouseau-app's own `CLAUDE.md` ("Testing Standards") mandates a spec-driven workflow for this repo,
and it applies here: `/write-spec <file>` scaffolds a spec from a component's public API, a human
reviews and approves it, `/test-spec <spec-file>` generates tests from the approved spec, then
`/test-validate` checks quality and coverage. **Tests are never generated directly from source
code** — this is a hard rule in this repo, unlike the Rust side of this same body of work, where
tests were written directly alongside the implementation this session (appropriate there — gent-cli
has no spec-driven mandate). Do not carry that Rust habit into this repo's cutover work.

Concretely, before writing implementation code for each capability below, run `/write-spec` against
the files that capability's "Dart work" section lists as edited or deleted (e.g. capability 1 needs
a spec covering `local_git_service.dart`'s thinned read methods and `GentdWorkspaceGitReport`
parsing; capability 5 needs one covering `checkpoint_controller.dart`'s new capture/restore calls
and the `panel_dialogs.dart` confirmation wiring), get it approved, then `/test-spec` it. Tests land
under the existing `app/test/` convention: `app/test/unit/` for pure `GentdIpcClient` method
additions and model `fromJson` parsing, `app/test/widget/` for anything touching
`panel_dialogs.dart`/`panel_models.dart` UI, `app/test/integration/` for a full local-request or
local/remote-parity round trip. Use the existing `app/test/helpers/` factories, fakes, and
`buildTestApp()` — do not hand-roll `RefenaScope`/`MaterialApp` boilerplate or duplicate stubs.

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
branch and `_findSubReposLocal` get replaced by the capability call (local) / existing REST route
(remote). **No new route needed**: `lib/provider/network/server/controller/git_controller.dart`
already exposes `GET /api/mate/v1/git/info`, `/git/remote-status`, `/git/repo-root`, and
`/git/sub-repos` — these are the current remote-device read surface, backed today by shelling git
directly. Re-point their handler bodies to call `GentdIpcClient.workspaceGitStatus`/
`.workspaceGitSubRepos` instead; the route paths, params, and response shapes native already
consumes remotely do not need to change.

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

**Local/remote**: extend the existing `PATCH /api/mate/v1/agent-chat/:id/settings` route
(`lib/provider/network/server/controller/agent_chat_routes_history.dart`) rather than adding a new
one — it is already the "adjust this conversation's launch settings" resource (adapter/model/effort
today). Add the config fields (`systemPrompt`, `appendSystemPrompt`, `maxTurns`,
`disallowedTools`, `revision`) to its accepted body, backed by `saveConversationConfig` instead of
local persistence. Verify whether a `GET` counterpart for reading current settings already exists
before adding one — if the only current read path is embedded in the full conversation-detail
fetch, a config read may not need its own route at all.

**Acceptance**: setting a system prompt on a Claude conversation changes its next turn's launched
arguments; the same field on a Codex conversation shows as unsupported in the UI rather than being
silently accepted and ignored; a stale-revision save (e.g. two devices editing at once) is rejected
and the UI reloads the current record rather than clobbering it.

Capabilities 4–6 continue in `scoped-six-capability-cutover-plan-part-2.md`:

4. Fork — `agent-chat-intents-v1` (existing capability, new variants)
5. Checkpoint — `agent-chat-checkpoint-v1`
6. Side questions — `agent-chat-side-question-v1`
