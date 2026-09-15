# Gent continuation handoff

## Objective

Make `gent-cli`/Gentd the single Agent Chat product authority. The standalone `gent` terminal and the Gent native Flutter app must use the same daemon, durable conversations, sessions, provider lifecycle, tool/activity timeline, permissions, MCP, local models and workspace state. Native integration is deferred until standalone Gent is complete and verified; the current work was an integration-readiness audit and plan, not Flutter implementation.

## State at handoff

The integration plan was expanded and corrected under `PLANS/INTEGRATE_GENT_CLI_TO_NATIVE_APP/`. Read its `README.md` first. No application code was changed in this planning pass and nothing was committed or pushed.

The repositories are already heavily dirty. Treat existing changes as user work; do not reset, discard, reformat broadly, commit or push without explicit approval.

## Decisions

- Gentd is the only durable and provider-facing authority. `gent` and Flutter are presentation clients, never provider adapters or parallel chat stores.
- Flutter must be generic and descriptor-driven. It renders Gentd catalogs/projections/actions with stable IDs; it must not hard-code provider/model/effort/mode/permission/tool vocabularies. New Gent catalog entries must appear without a Flutter release.
- The native app must remove `default`, `autonomous`, `auto-accept edits`, and `bypass permissions` as mode choices. Gent mode is initially `Ask`, `Plan`, `Agent`; permissions are independent and Gent-owned. The terminal/core currently still exposes the old permission posture vocabulary, so fix Gent first, then have Flutter render its catalog.
- Recommended permission domain: a separately versioned, workspace-scoped category policy with expected-revision conflict handling. It is not part of `AgentChatSelection`, which remains provider/model/effort/mode.
- Use one platform-standard shared Gent data directory from a Rust resolver, not Flutter `~/.gentd` versus CLI `~/.gent-cli` defaults. `GENT_DATA_DIR` is only an explicit override for development/testing.
- Native packages must consume one verified Gent release archive intact (`gent`, `gentd`, Node/npm, Claurst, llama.cpp), rather than independently choosing runtime versions in the native repo. The app-bundled runtime is app-updater-owned; terminal self-update must not mutate a signed app bundle.
- Claurst and llama.cpp are bundled but model weights are not. A missing curated ungated model downloads only after an accepted durable prompt. Claude/Codex install only after an accepted prompt selects that provider, not merely on selection. Authentication retains that prompt and releases it once after verified readiness.
- Canvas, STT, draft/focus/scroll and picker transients may remain native-local. They can only produce normal Gentd intents/artifacts; no alternate agent path.
- Companion mobile clients cannot access desktop IPC directly. The desktop host must proxy typed Gentd restore pages/deltas/intents through the authenticated remote transport.

## No-snapshot contract

- Snapshot state, recovery caches, mirrored state, and replacement layers are prohibited. Gentd's durable ledger facts are the only authority; every client view is a fold of those facts through one shared reducer.
- Derived views are disposable/non-authoritative: never serialize, persist, transmit as truth, or use them to recover or reconnect.
- On connect, an epoch change, a follow error, or an unaccepted cursor, a client must reload immutable bounded pages (from cursor zero when needed), then replay normalized facts strictly after the accepted cursor.
- `agent-chat-projection-v1` `conversationSnapshotRequest` is a read, not stored state: Gentd returns conversation detail, the current goal record (`goal`), the projection cursor, and a bounded tail of at most 100 transcript and 100 activity facts through that cursor, skipping partial transcript facts superseded by a later final fact of the same turn and kind; `transcriptTruncated`/`activityTruncated` mark a clipped tail. Nothing from it is written back.
- The native restore uses that read only for detail, goal and cursor. It pages the transcript from cursor zero and each run's activity from zero, folds them with `gentd_projection_reducer.dart`, then follows projection deltas after the read's cursor. Remote viewers receive the same pages over mux and reduce them with the same reducer.
- Committed facts are immutable. Gentd never updates or deletes a committed transcript, activity, lifecycle or projection fact, so every cursor a client accepted stays valid and no compaction fence exists; any future physical pruning must first introduce a fence exposed to clients and answer a follow or page request below it with a typed resync-required response. Settlement supersedes instead of rewriting: before Gentd appends a non-ready terminal fact for a turn, it appends one final transcript fact per streamed kind (`interrupted:<last partial event id>`) carrying the unfinished partials after that kind's last final, at a new cursor ahead of the terminal fact. A partial is superseded when a later final of the same turn and kind exists; transcript pages (`agent-chat-transcript-v1`) and the snapshot tail skip superseded partials through the `(turn_id, kind, is_partial, cursor)` index, while projection follows deliver every fact, so a live follower and a client restored from pages reduce to the same view.
- Context compaction appends, never replaces: a Gent summary is an immutable `agentChatContextCompaction` event plus a transcript notice, valid only while the digest of the entries it covers still matches the run's view; the full ledger stays authoritative and every rendered provider input remains a disposable derivation (`docs/providers/claurst.md` "Context compaction").
- The native client keeps its fold, including `gentdProjection`, in memory only; `AgentChatState` never serializes a projection position. A session follows projection deltas only from a position restored from pages in the current app run (a hidden tab may resume from that in-memory cursor); on restart, reconnect, eviction, a follow error or resync-required it restores from pages first. The legacy local conversation store still writes message rows for Gentd conversations as sidebar metadata; restore replaces them and they never seed a cursor or a follow, and they are deleted with that store at cutover.

## Important discoveries and gotchas

- The native bridge already exists but is not the target: `clouseau-app/app/lib/provider/agent_chat/agent_chat_gentd.dart` reconstructs Flutter `ChatMessage` objects, injects a synthetic history envelope on first conversion, polls permissions/activity separately, and filters model downloads by model ID. Delete this authority during cutover; it is not safe to extend.
- Native `GentdAppRuntime` currently defaults to `~/.gentd`, while CLI/Gentd default to `~/.gent-cli`; they do not share state by default. It also spawns before connecting, so it mishandles a daemon already owned by `gent`.
- Current public transcript/activity facts are not sufficient for the native timeline: they lack complete tool input/output/content-block/diff and child task/output detail. Add typed work-item records with stable IDs and bounded output paging before Flutter refactoring.
- `agent-chat-projection-v1` is still prose. Existing Flutter IPC client exposes separate conversations/transcript/turn/activity/permission/attachment/model calls. Define restore-read/delta ordering, cursors, resync, paging, receipts, capabilities and unknown descriptor rendering.
- Provider auth is not yet a real Gentd lifecycle. `/login` currently runs a separate one-off Gentd command; `provider-auth-v1` is not composed by `RuntimeFacade`. Native needs a generic external-auth descriptor for URL/device-code/provider-terminal handoff.
- Local-model download cancellation is subtle: one underlying model download can serve multiple prompt admissions. Cancel must detach one prompt receipt, not stop work for another waiter. Request ID plus model ID is mandatory.
- Gent release archives already contain `gent`, `gentd`, `runtime/node`, and `runtime/claurst`; native packaging currently stages runtime parts independently and omits `gent`. Internal MCP bootstrap may resolve `gent`, so this is functional as well as distribution work.
- Current release matrix lacks Windows ARM64 Claurst/llama runtime. v1 plan requires macOS x64/ARM64, Linux x64/ARM64 and Windows x64/ARM64.
- Fork/resume/checkpoint, general MCP live config/health, session metadata, title/recap streaming, provider readiness/install, and remote proxying lack complete public contracts even where partial internal types exist.
- Do not reimplement provider behavior in Flutter. Claude/Codex/Claurst remain third-party runtimes; verify Gent-owned admission, context-switch, durable-state and projection behavior.

## Plan documents

- `PLANS/INTEGRATE_GENT_CLI_TO_NATIVE_APP/README.md`: reading order and native-integration entry gate.
- `gentd-source-of-truth-contract.md`: generic native client rule.
- `integration-gap-review.md`: audited blockers and exact evidence.
- `implementation-backlog.md`: priority-ordered work packages and acceptance gates.
- `native-surface-disposition.md`: every native Agent Chat control and its final owner.
- `native-agent-chat-cutover-map.md`: Flutter cutover/deletion boundary.
- `onboarding-and-provider-readiness-contract.md`: runtime packaging, prompt admission, install/login and cancellation semantics.

## Remaining work, in priority order

1. Implement shared Gent path resolution and safe connect-first multi-client daemon supervision.
2. Finalize Gent-owned independent permission policy/catalog and update standalone terminal/core away from old permission-mode vocabulary.
3. Build `agent-chat-projection-v1`: descriptor catalogs, complete typed work items, bounded restore reads/deltas, cursors, output paging, receipts and capability/version negotiation.
4. Implement provider/local-model admission lifecycle: request-scoped download/install/auth progress, cancellation and exactly-once held-prompt release.
5. Complete authority APIs for sessions, titles/recaps, templates/docs, MCP, Git, automations, forks/resume/checkpoints and remote desktop proxy.
6. Package one full Gent release runtime in each native desktop bundle and installer shim, including the Windows ARM64 target.
7. Only then replace Flutter Agent Chat authority with the generic Gentd client and delete existing mirror/adapter state.
8. Run real cross-client user flows, focusing on Gent-owned behavior and reconnects rather than duplicating upstream provider tests.

## Working rules

- Prefer reuse/porting of proven native behavior, but move durable ownership into Gentd instead of copying Flutter code.
- Fixes must simplify the system; prefer removal/consolidation over flags, layers or special cases.
- Do not add comments, docblocks, TODO/FIXME notes, lint/type suppressions, or commented-out code. Use names, structure and tests; rationale belongs in commits/PRs. Shebangs are allowed.
- Do not introduce legacy routes, compatibility branches, migrations, direct-provider fallbacks or dual durable writers.
- Do not commit or push without the user's explicit approval.

Resume here: Read `PLANS/INTEGRATE_GENT_CLI_TO_NATIVE_APP/shared-data-dir-and-daemon-ownership.md` first — it records what backlog item 1 has completed (a single `gent_types::paths` resolver used by `gent-cli` and `gentd`; the `gent data-dir` / `gentd --print-data-dir` offline bridge; owner-naming on `gentd.lock` conflicts) and what remains: the deferred `Negotiated`/`HostStatus` handshake extension (blocked on inspecting `gentd_ipc_client.dart`'s frame decoder first) and the three specific Flutter-side defects it found but did not fix (`GentdAppRuntime.dataDirectory()` defaulting to `~/.gentd` instead of the shared resolver; `_launch` always spawning instead of connect-first; `app/rust/src/api/gentd_ipc.rs`'s untested Windows pipe-name test). Start the next unit of work by inspecting `gentd_ipc_client.dart`'s JSON decoder to decide whether the handshake extension is safe, or — if ready to begin the Flutter cutover for item 1 specifically — fix `GentdAppRuntime` per that document. Do not touch other Flutter Agent Chat surfaces; that is backlog item 7, gated on items 2-6.

Also noted, unrelated to this item and not fixed: three pre-existing failing tests in the user's uncommitted WIP, unrelated to any change made in this pass — `gent-types::prompt_templates::tests::{rejects_missing_and_repeated_variables,renders_bounded_named_variables}`, `gent-store::sqlite::normalized_session_ledger_tests::{batch_commits_all_projections_with_exact_retry_cursors,batch_collision_rolls_back_every_prior_projection}`, and `gent-testkit`'s `tests/ipc_fixture_manifest.rs` (`repository_ipc_fixture_contract_is_valid`, `agent_chat_contract_cannot_be_declared_composed`). None of the files involved were touched in this pass; confirmed pre-existing before this session started.
