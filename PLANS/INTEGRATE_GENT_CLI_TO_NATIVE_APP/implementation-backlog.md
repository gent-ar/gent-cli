# Integration implementation backlog

Ordered work plan. Gent core and standalone complete first; native integration begins only after the
corresponding Gentd contract exists and is exercised by the terminal.

Path convention matches `integration-gap-review.md`: `crates/…` and `tools/…` are in this repo;
`native:…` is `/Users/ivanmatiasfort/Clouseau/clouseau-app`. Evidence for every claim below is in
`integration-gap-review.md`.

## 0. Commit the untracked Rust modules

`crates/gent-types/src/paths.rs`, `agent_chat_sessions.rs`, `automations.rs` and
`prompt_templates.rs` are untracked; `observer_tap.rs` is deleted but uncommitted. Everything in
item 1 depends on them.

Acceptance: `git ls-files crates/gent-types/src/paths.rs` prints the path, and
`cargo test --workspace` passes from a fresh clone of the committed tree.

## 1. Shared data directory and daemon authority

Rust is complete: `gent_types::paths` is the one resolver (canonical directory `<home>/.gentd`, with
a one-time migration from the old `<home>/.gent-cli` — decided and built, see
`integration-gap-review.md`), `gent data-dir` and `gentd --print-data-dir` expose it to external
hosts, `host_lock.rs` names the conflicting owner, and `local_ipc.rs::connect_or_start` is a working
connect-first client. The remaining work is in the native repo, two Rust de-duplications, and the
shutdown-policy item below.

1. DONE. `GentdAppRuntime.dataDirectory()`
   (`native:app/lib/service/gentd/gentd_app_runtime.dart`) now resolves via one cached invocation of
   the bundled `gentd --print-data-dir`, passing no `--data-dir`; the independently-computed
   `<home>/.gentd` and app-support fallback branches are deleted (Flutter no longer computes any
   directory name itself, so it needed no further change when the canonical name below was decided).
2. DONE. `GentdAppRuntime._launch()` is connect-first, mirroring `crates/gent-cli/src/local_ipc.rs:113`:
   `client()` calls `_ensureAvailable`, which tries a connection (`_canConnect`) before ever calling
   `_launch`/spawn; `_launch`'s poll loop races the spawned child's `exitCode` (mirroring
   `wait_for_connection_until`'s `child.try_wait()`) and surfaces its captured stderr on failure. The
   cached "available" future is cleared only by `reportUnavailable()`, called from the owned
   process's own exit handler or a failed attempt — never from an externally-owned daemon's activity.
   Covered by `native:app/test/unit/gentd_app_runtime_test.dart`.
   Not covered by this change, and still open: if a daemon Flutter *connected to but did not spawn*
   dies mid-session, nothing currently calls `reportUnavailable()` to trigger re-verification (no RPC
   failure path is wired to it yet). `reportUnavailable()` exists as the hook for that; wiring it up
   is the remaining piece of the third acceptance line below.
3. Add `gent-types` as a path dependency of `native:app/rust` and call
   `gent_types::windows_pipe_name` / `local_socket_path` from
   `native:app/rust/src/api/gentd_ipc.rs`. Delete its local `endpoint_hash`, `pipe_name` and the
   `pipe_name_matches_gent_cli_derivation` test, whose asserted hash is wrong. Deferred from the item
   1/2 change above because it is a cross-repo Cargo dependency: confirm `gent-types`'s dependency
   tree (notably `directories`, used only for `BaseDirs::home_dir()`) builds cleanly for
   `app/rust`'s iOS/Android targets before wiring it in, and decide path-dependency-on-a-sibling-repo
   versus a pinned git dependency for CI reproducibility.
4. Replace `gent_cli_executable()` (`crates/gentd/src/standalone_mcp_config.rs:266`) with
   `gent_types::resolve_sibling_binary("gent")`.
5. Add `daemon_version: String` to `Negotiated` (`crates/gent-protocol/src/lib.rs:124`) and
   `HostStatus` (`crates/gent-types/src/lib.rs:256`), and surface it through
   `native:app/lib/service/gentd/gentd_ipc_client.dart`'s `_open()` so a version mismatch reports an
   upgrade requirement. This is safe to do now: the Dart decoder ignores unknown object keys
   (`_object` casts without key validation; `_open` reads only `protocol` and `capabilities`), so the
   field is additive on the wire.
6. Decided (see `integration-gap-review.md`): gentd shuts itself down after an idle grace period with
   no client presence, regardless of who spawned it — not on any one client quitting. Implement
   presence as a rolling last-activity timestamp bumped on every accepted request, plus any currently
   open long-lived subscription (`event-stream-v1`, `agent-chat-turn-follow-v1`) counting as presence
   for its whole open duration. Confirm during implementation whether the native app already keeps a
   subscription open whenever its window is open; if not, add one for this purpose. Release
   `host_lock` cleanly on self-exit. No client-side kill/shutdown call is needed on either `gent` or
   the native app.

Acceptance:
- `gent data-dir` and the app's resolved directory print the same string on macOS, Linux and Windows.
- Start `gent`, then launch the app: exactly one `gentd` process exists for that directory
  (`pgrep -f gentd | wc -l` is 1), and it stays 1 after ten app operations. DONE — proven by
  `client() never spawns when a daemon is already reachable` and the 20-call regression test in
  `gentd_app_runtime_test.dart`.
- Quit `gent` and the app while a `gentd` they share is idle: it exits within one grace period of the
  last one disconnecting, and never exits while either still has presence.
- Kill the daemon while the app runs: the app reconnects or spawns exactly one replacement. NOT YET
  MET for a daemon the app connected to without spawning — see the note under item 2.
- A `gentd` older than the app's required `daemon_version` produces a visible upgrade message, not a
  degraded session.

## 2. Gent-owned display catalogs

`AgentChatMode` is already `Ask | Plan | Agent` (`crates/gent-types/src/agent_chat.rs`), and
`permission-policy-v1` already exists with a workspace-scoped, revisioned `PolicyRecord`
(`policy_id`, `workspace_id`, `scope`, `revision`, `mode`, `allowed_tools`, `allowed_categories`) and
explicit-revision writes (`crates/gent-protocol/src/permission_policy.rs`). The mode/policy split is
built. What does not exist is any display record: the whole vocabulary is bare Rust enums with no
label, ordering, availability or explanation.

1. Rename `PermissionMode::Plan` (`crates/gent-types/src/policies.rs:18`). It is the only posture
   value that collides with an `AgentChatMode` name. Rename in place; no alias, no compatibility
   mapping.
2. Add one catalog capability publishing a single generic record type reused by every vocabulary:
   `id`, `label`, `ordering`, `available`, `unavailable_reason`, `explanation`,
   `requires_confirmation`, `scope`. Address catalogs by catalog ID — `provider`, `model`, `effort`,
   `mode`, `permission-policy`, `tool-source`, `composer-action` — so a new vocabulary adds an ID,
   not a frame type. Add it to `DECLARED` in `crates/gent-runtime/src/catalog.rs:44`.
3. Keep selection as `AgentChatSelection` (provider/model/effort/mode). Permission policy stays a
   separate `PolicyRecord` write with expected-revision conflict handling.
4. Make the terminal render and mutate only catalog records.
   `crates/gent-cli/src/terminal/state_permissions.rs` currently parses the literals
   `ask | read | edits | autonomous | bypass confirm`, and
   `crates/gent-cli/src/terminal/render_composer.rs:109` displays a *different* set —
   `ask | read-only | auto edits | autonomous | bypass`. Delete both literal sets; the parser matches
   catalog IDs and the renderer prints catalog labels.

Acceptance:
- `crates/gent-cli/src/terminal/` contains no permission or mode string literal; grep for `"bypass"`,
  `"autonomous"`, `"auto edits"` in that directory returns nothing.
- Adding one catalog entry in Gentd makes it appear and be selectable in the terminal with no
  terminal code change, proven by a test that adds an entry and asserts it renders.
- Selecting a policy whose category set a provider cannot express returns a typed unavailable result
  carrying `unavailable_reason`, not a silently widened policy.

## 3. `agent-chat-projection-v1`

1. Define one snapshot + ordered-delta contract for conversation and workspace. It replaces and
   deletes six existing capabilities: `conversation-index-v1`, `agent-chat-conversations-v1`,
   `agent-chat-transcript-v1`, `agent-chat-turn-follow-v1`, `conversation-activity-v1`,
   `agent-chat-sessions-v1`. `agent-chat-intents-v1`, `agent-chat-permissions-v1`,
   `attachments-v1`, `local-models-v1` and `permission-policy-v1` remain as mutation surfaces.
2. Add a work-item read for content that the activity stream deliberately omits: tool input JSON,
   tool output body, diff, content blocks, command output, child task text and child model, keyed by
   `tool_use_id` / `work_id` / `child_id` and correlated to `ConversationActivityFact` cursors. Do
   not widen `ToolActivity` (`crates/gent-types/src/tool_activity.rs:30`) — it rides the hot stream.
3. Page transcript, outputs, diffs, attachments and child detail with bounded page sizes.
4. Define one total cursor order across transcript, activity and workspace streams, plus snapshot
   watermarks, resync-on-gap behavior and idempotent mutation receipts.
5. Publish status descriptors for loading/thinking, download, install, auth, command, subagent,
   attention and terminal outcomes as catalog records from item 2, not new enums.

Acceptance:
- The terminal renders every work surface from the projection alone; grep shows no screen deriving
  tool or child state from transcript text.
- The six replaced capabilities no longer appear in `crates/gent-runtime/src/catalog.rs`.
- Replaying a follow stream from a stale cursor yields the same final state as a fresh snapshot, for
  the same conversation, asserted by a test.
- Sending the same mutation receipt twice produces one effect and two identical receipts.

## 4. Provider and local-runtime admission

1. Add `provider-auth-v1` to `crates/gent-runtime/src/catalog.rs`. The constant
   (`crates/gent-protocol/src/provider_auth.rs:15`) and the handler
   (`crates/gentd/src/provider_auth_transport.rs:27`) already exist, but the capability is never
   declared, so negotiation can never reach the handler. Then delete the one-off Gentd process that
   `gent`'s `/login` starts as a workaround.
2. Extend the auth frame with request ID, progress, cancellation, retry and exactly-once release of
   the held prompt receipt.
3. Start Claurst and llama.cpp only for an admitted local prompt; release them when runtime work
   ends. Both already ship in the release archive under `runtime/claurst/`.
4. Keep model weights out of the installer; download only after a durable prompt is accepted.
   Correlate every download by request ID as well as model ID — concurrent prompts may await the same
   model.
5. On a Claude or Codex prompt, install the managed CLI only if absent, emitting install progress
   under its own request ID.

Acceptance:
- A first prompt to each provider survives download, install and login without being retyped,
  duplicated, or disappearing, verified once per provider.
- Cancelling one conversation's download leaves a second conversation waiting on the same model
  still waiting, and the underlying download running.
- `gent /login` spawns no second `gentd` process.

## 5. Durable product domains

`prompt-templates-v1`, `workspace-documents-v1`, `automations-v1`, `forge-connectors-v1`,
`reviewed-plan-v1`, `goal-v1` and `orchestration-v1` already exist in
`crates/gent-runtime/src/catalog.rs`. This item completes them rather than creating them.

1. Audit each existing capability against the rail it must feed in
   `native-surface-disposition.md`, and add only the missing fields — titles, recaps, previews,
   attention state, ordering, run history.
2. Add fork, resume and checkpoint intents to `agent-chat-intents-v1`. Filesystem restore needs a
   confirmation receipt distinct from the restore intent.
3. Extend `forge-connectors-v1` to cover every MCP source, not only Forge-generated ones: config
   source registration, live updates, health, credential ownership, per-conversation selection and
   reconnect semantics. Make `<data_dir>/standalone-mcp.json`
   (`crates/gentd/src/standalone_mcp_config.rs`) one registered source rather than the mechanism. Do
   not add a parallel MCP capability.
4. Define smart-metadata provenance and revision. The native app requests a title after the first
   assistant completion and a recap at completions 6, 12, 18…; Gentd must own that schedule for all
   three providers.
5. Add `agent-chat-side-question-v1` (decided: `/btw` survives — see `integration-gap-review.md` for
   the full spec). Port the bounded-excerpt rule from
   `native:app/lib/util/side_question_context.dart` (8 messages, 12000 chars) into a shared Rust
   helper. Add a streaming, cancelable `ConversationSideQuestionRunner`, reusing
   `SystemLauncher`/`ProviderLaunch` from `claude_summary_runner.rs` and its Codex/Claurst siblings
   rather than a new process-spawn path. Enforce the 3-per-conversation/8-total-live concurrency caps
   and the 300s timeout in Gentd, not per-client. Publish `side_question_begin/delta/end` on the same
   event stream and cursor order as `agent-chat-projection-v1`. Delete
   `native:app/lib/provider/network/server/controller/agent_chat_btw.dart` and its Dart-side
   concurrency maps in the same native cutover that adds the `ask`/`cancel` intents.

Acceptance: terminal and native show the same sidebar rows, in the same order, for the same
workspace, and neither writes a durable copy of them. A side question asked from native and one asked
from the terminal against the same daemon share one concurrency budget.

## 6. Package one Gent runtime

1. Delete the `gent`-dropping filter (`native:tools/stage-gentd.py:129-133`) and the runtime
   overwrite (L141-148). Stage the downloaded archive intact under one runtime root.
2. Collapse `native:mate.json`'s four independently versioned `bundled` entries (`nodejs`, `claurst`,
   `llama_cpp`, `gentd`) into one `gentRuntime` entry: version, repo, per-target sha256.
3. Generate the archive manifest's `capabilities`
   (`tools/package-release.py:137-144`) from
   `gent_runtime::catalog::declared_capabilities_with_profiles()`. The hand-written seven-entry list
   already omits three capabilities the Dart client requires.
4. Add `aarch64-unknown-linux-gnu` to this repo's release matrix
   (`.github/workflows/release.yml:89-124`) — `tools/stage-claurst-runtime.py:27-30` already supports
   it, and consuming an intact archive without it would regress Linux ARM64.
5. Publish or build a Windows ARM64 Claurst and llama.cpp runtime, add `aarch64-pc-windows-msvc` to
   the release matrix and `stage-claurst-runtime.py`, then remove the
   `"Windows ARM64 Claurst runtime is not published"` throw at
   `native:app/windows/Scripts/download_claurst.ps1:16-22` and its assertion at
   `native:tools/test-claurst-download-scripts.py:40`.
6. Stage both macOS targets: `native:.github/workflows/release.yml:241` passes only
   `--target aarch64-apple-darwin`.
7. Add `resolveGent()` to `native:app/lib/util/bundled_runtime.dart` and install rules for `gent` in
   `native:app/macos/Scripts/bundle_claurst.sh`, `native:app/windows/CMakeLists.txt:127-135` and
   `native:app/linux/CMakeLists.txt:159-168`, plus the installer-owned `gent` command shim.
8. Make the app updater the sole owner of the bundled `gent`/`gentd`. `runtime-update-check-v1` must
   report an available update inside an installed bundle and refuse to mutate it.

Acceptance:
- Release CI produces six archives: macOS x64/ARM64, Linux x64/ARM64, Windows x64/ARM64.
- On each, a fresh install runs `gent --version`, completes a `gentd` handshake, and resolves the
  packaged Claurst — asserted by a release check, not by hand.
- `native:mate.json` contains exactly one bundled runtime version.
- After install, `gent` and the app open the same conversation from the same directory.

## 7. Flutter cutover

1. Keep the FRB byte transport unchanged. Add the projection client types to
   `native:app/lib/service/gentd/gentd_ipc_client.dart` first, with no UI change.
2. Replace `native:app/lib/provider/agent_chat/agent_chat_gentd.dart` with one snapshot/delta
   reducer producing presentation view models. It contains no provider process, protocol parser,
   transcript persistence or permission engine. This reducer is the `GentdAgentChatController`
   referenced in `native-agent-chat-cutover-map.md`; the two names denote one class.
3. Delete, in the same change: `gentdHistoryEnvelope` (`agent_chat_gentd.dart:10`), the
   transcript-to-`ChatMessage` reconstruction (L616-626, L685), the `selectedModel`-keyed download
   filter (L487, L501), the out-of-band permission refetch (L99, L173-189, L420), and
   `agent_chat_adapters.dart`, `agent_chat_turn.dart`, `agent_chat_send_dispatch.dart`,
   `agent_chat_stale_processing.dart` for Gentd conversations.
4. Convert every surface in `native-surface-disposition.md` to catalog records and typed intents.
5. Keep only presentation-local state: selected rail, modal visibility, draft text, focus, scroll,
   keyboard, canvas, STT.

Acceptance:
- Adding a catalog entry in Gentd makes it appear in the native UI with no Flutter rebuild.
- Restarting the app during a running turn loses and duplicates no event, verified by comparing the
  post-restart transcript to the daemon's.
- No file under `native:app/lib/provider/agent_chat/` writes durable conversation state.

## 8. Remote companion path

1. On desktop, proxy Gentd snapshots, deltas and intents through the existing authenticated mux
   transport as an opaque typed passthrough. Forward no raw provider command and no local endpoint
   path.
2. Point mobile Agent Chat at that proxy, replacing
   `native:app/lib/provider/agent_chat/agent_chat_remote.dart`'s state access.
3. Preserve capability negotiation, receipt correlation and resync rules across the proxy unchanged.

Acceptance: a companion resolves a permission and stops a command on a desktop conversation, and the
desktop reflects both, without the companion running Gentd.

## 9. Final user-flow validation

Run once per desktop platform, from a fresh install:

- `gent` and the app share one conversation and session history.
- Local model download, cancellation and resume.
- Claude and Codex install and login from a held prompt.
- Model, provider, effort, mode and permission-policy switches mid-conversation with preserved
  context and no UI reversion.
- Attachments, MCP, Git/worktree, command output, subagents, permissions, plans.
- Fork, resume, checkpoint restore.
- Remote companion control.

Verify Gentd-owned effects only; do not re-test provider internals the upstream CLI already
guarantees.
