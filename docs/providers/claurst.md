# Claurst — provider knowledge map

Claurst is an internal ACP-based provider. Its credentials/endpoints/routing must never appear
in public Gent (`gent-core`, `gent-drivers`, `gent-adapters`, `gent-types`) — verified: those
crates contain only a generic `"claurst"` argv literal in `D:launch_spec.rs:160` and test
assertions in `crates/gent-adapters/src/provider_platform.rs`. All real Claurst logic lives in
`crates/gentd/src/` (private), which is correct per CLAUDE.md's non-negotiable architecture.

Pinned: `providers.claurst` in `fixtures/provider-contracts/pins.json` (release tag `v0.1.7`,
per-target sha256 for the actual shipped binary tarballs) plus `runtimes.llama_cpp` (tag
`b10545`, per-target sha256 — Claurst bundles a `llama-server` runtime alongside its own binary).
Snapshot: `fixtures/provider-contracts/claurst/0.1.7/` (`source.json`, `commands.json`,
`protocol.json` — no `launch-probe.json`; `source.json` has `"evidence": "sourceCheckout"` and
`"pinned_release_binary_verified": false"`, because the *protocol* snapshot is captured by
introspecting a source checkout + the locked `agent-client-protocol-schema` crate, while the
*shipped runtime* is the separately pinned, sha256-verified release tarball — these are two
different trust paths for the same version, not a contradiction).

Abbreviations: `G:` = `crates/gentd/src/`.

## Concern → file → symbols

| Concern | File(s) | Key symbols |
|---|---|---|
| Launch argv/env | `G:claurst_local_runtime.rs::ClaurstLocalRuntimeRequest`, `LocalProcessLaunch`, `ClaurstLocalRuntimePlan`, `build()` (62) — argv plus settings JSON<br>`G:claurst_local_profile.rs` — `template_kwargs()`, `history_input_bytes()`, `COMPACT_TOOLS` const (Plan drops `Edit`/`Write` and adds `Agent`/`monitor`), per-mode prompt guidance<br>`G:claurst_runtime_factory.rs::ReadyClaurstRuntime`; `G:standalone_claurst_runtime_factory.rs::{new(),bridge()}` |
| Protocol wire parsing | `G:claurst_acp_transport.rs::ClaurstAcpTransport<S>` — `new()`, `initialize_session()`/`initialize_session_with_mcp()`, `prompt()`/`prompt_content()`, `cancel()`, `drain()`, `respond_permission()`, `supports_images()`<br>`G:claurst_acp_transport_io.rs` (`pub(super)`) — `send_request()`, `wait_for_response()`, `read_frame()`, `write()`, `handle_frame()`, `unanswered_stop_reason()`<br>`G:claurst_acp_transport_updates.rs::session_update_fact()` (8) dispatches `sessionUpdate` kind — **verified: unmatched kinds fall to `_ => None` and are silently dropped** (line 21) |
| Send now / interrupt | Send now = interrupt, as on every provider; queued prompts never join a Claurst turn. `G:claurst_acp_transport.rs::cancel()` sends `session/cancel` and answers held permission requests `cancelled`; the turn ends only on Claurst's own `session/prompt` answer `stopReason: cancelled` (no timer). Observed on 0.1.7+gent.mcp1 with qwen3-1-7b: the answer arrives within milliseconds of the cancel. Claurst does not re-arm a session's cancel token, so every later prompt on a cancelled session answers `cancelled` at once; `G:claurst_acp_bridge_operations.rs::cancel_blocking()` therefore drops the run's ACP session and the next prompt opens a fresh one. An exit before the answer fails the turn through the drain. The runtime launch (`ensure_for_prompt`: llama-server, Claurst, model load) runs as its own task (`G:claurst_prompt_lifecycle/launch.rs`, `Work::Launching`) unless the runtime is already warm; an interrupt during it removes the launch and settles the turn `interrupted` through `interrupt_unstarted_agent_chat_prompt`, and the launch finishes in the background as a warm runtime. Interrupts reach the lifecycle as soon as it is free (`ordinary_lifecycle_cadence_claurst.rs::interrupt_claurst`); warm-ups are detached too. Claurst flushes every event it already generated before its `cancelled` answer, so the turn ends only after Gentd has recorded them: each drain (up to 64 facts) is recorded in one transaction (`private_claurst_ingress_projection.rs::record_facts`, `append_normalized_session_batches`), not one synced commit per token, so recording keeps pace with generation. Reaping an idle runtime is detached like warm-up (`claurst_prompt_lifecycle.rs::reap_idle`), so stopping a process tree never holds the lifecycle an interrupt waits for |
| Background commands | Contract in `docs/background-commands.md`. `G:claurst_acp_transport_background.rs` owns the open set: at a `completed` answer every tool still `in_progress` (and not holding a permission the answer cancelled) becomes `background` and survives the next `session/prompt`; its later `tool_call_update` maps `completed`, `failed`+`rawOutput.status: "cancelled"` → `interrupted`, other `failed`; a Claurst exit ends them `failed`; every other stop or replacement of the runtime ends them at release (`StandaloneClaurstRuntimeFactory::stop_conversation`, the relaunch in `launch_session`) through the store rule in `docs/background-commands.md`, `interrupted` after a Stop. `drain()` keeps reading stdout without a pending prompt only while one is open (woken by the stdout reader, no timer). `G:claurst_prompt_lifecycle/background.rs` follows the ended turn's source (`PrivateClaurstBridge::runs_background_commands`, `G:private_claurst_ingress_background.rs`) and records the end there; the store files it under the launching turn. `cancel_blocking()` accepts an ended source while background commands are open and sends `session/cancel` for each of their sessions that belongs to the stopped run (each conversation has its own Claurst process, so another conversation is never touched), so Stop with no running turn ends them; the run's ACP session is dropped, so the next prompt never meets the un-re-armed cancel token. Transport `is_idle()` is false while one is open, so the runtime is not reaped or evicted |
| Permission relay | `G:claurst_permission_policy.rs::decide()` (20)<br>`G:claurst_acp_transport_updates.rs::{OpenTool,PermissionTool,claim_permission_tool()}` (98/106/113) correlate a `tool_call` update to a pending permission<br>`G:private_claurst_ingress_permission.rs`, `G:private_claurst_ingress_permission_decision.rs`<br>Typed delivery: `PortError::NotPending` (not pending, nothing to do), `PortError::InvalidInput` (answer not offered; the request stays pending, the receipt is rejected), anything else is a broken pipe and cancels the run. Requests still open when the prompt answers are answered `cancelled` by `G:claurst_acp_transport_prompt.rs::settle_permissions()`; a failed write there fails the drain and releases the runtime |
| Usage/token parsing | `G:claurst_acp_transport_updates.rs` — the `"usage_update"` arm of `session_update_fact()` calls a private `usage()` helper in the same file |
| Model listing | `crates/gentd/models.json` (the curated catalog data file — confirmed real, not `models.json` under `fixtures/`) plus user-picked GGUF files in `<data dir>/user-models.json`; contract and derivation in `docs/local-model-files.md`<br>`G:local_model_catalog.rs::{LocalModelCatalog,LocalModelRecord,LocalModelSource}`<br>`G:local_model_provisioning.rs::LocalModelProvisioner::{models(),model(),state(),add_user_model(),remove_user_model()}`<br>`G:standalone_authority_composition_claurst_models.rs::StandaloneClaurstModels` |
| Slash-command catalog | No Claurst-native commands — confirmed no `available_commands_update`, `CommandDescriptor`, or `slashCommand` reference anywhere under `claurst_*` files. The drift test's `claurst_findings()` will raise a `session-update-new available_commands_update` finding the day the pinned Claurst starts emitting it. Gent rows in `gent-core::agent_chat_commands` apply; `/compact` is the Gent `compact` intent (see "Context compaction") |
| Compaction | Gent-owned, see "Context compaction" below. `claurst_local_runtime.rs`'s `"auto_compact": true`/`"compact_threshold": 0.0` only configure Claurst's in-session compactor, which resolves an unknown `llama-cpp/*` window to a Claude-sized default and so never fires before llama-server's `--ctx-size`; Gent does not rely on it |
| Session recovery/resume fallback | No native resume: `G:claurst_acp_bridge_operations.rs::start_blocking()` (22) and `bind_blocking()` (113) always re-render history into the new session rather than resuming a provider-side one. `G:claurst_acp_bridge.rs::ClaurstAcpBridge<S>::with_history_input_bytes()` (57) bounds how much history is replayed |
| Provider upgrade rebind | No versioned-executable rebind path like Claude/Codex exists (Claurst is pinned by release tag + sha256, not a semver the daemon re-resolves at turn start). Restart-driven re-staging: `tools/stage-claurst-runtime.py` (dev/CI staging), `tools/bootstrap-dev-claurst-runtime.py` (dev bootstrap), `G:packaged_claurst_runtime.rs::{from_current_executable(),from_gentd_executable()}` |
| Install/provisioning, package policy | `tools/stage-claurst-runtime.py` — `pinned_artifacts()` (17, reads `pins.json` `providers.claurst.artifacts` AND `runtimes.llama_cpp.artifacts`, requires identical target sets), `digest()` (45, sha256), `extract()`/`extract_llama_runtime()`, `stage_claurst_source(...)` (140, the separate source-checkout path used only for protocol introspection, not for the shipped runtime)<br>`G:claurst_standalone_owner.rs`, `G:standalone_claurst_runtime_factory_launch.rs` |
| Native binary verification | Per-target sha256 against `pins.json`'s `providers.claurst.artifacts[target].sha256` and `runtimes.llama_cpp.artifacts[target].sha256` (two binaries, two digests). Readiness for the **model weights** (not the binary) is `G:claurst_local_readiness.rs::ClaurstLocalReadinessService::assess()` (39) via `LocalModelProvisioner` |
| Fake executables / test harnesses | No external fake-CLI script like Claude/Codex. Per-test-file in-process `Fake` structs implementing the `ClaurstAcpStdio` trait, e.g. `G:claurst_acp_transport_tests.rs` (`struct Fake`, line 11). Also `G:claurst_acp_bridge_tests.rs`, `G:claurst_acp_cancel_tests.rs`, `G:claurst_local_runtime_tests.rs`, `G:claurst_standalone_owner_tests.rs` |
| Contract snapshot / drift test | `tools/provider-contract-snapshot.py claurst --claurst-checkout <tag checkout> --acp-schema <locked crate dir>` (never runs the live binary).<br>`crates/gent-drivers/tests/provider_contract_drift.rs::claurst_findings()` — compares `protocol.json`'s `schemaSessionUpdates`/`emittedSessionUpdates`/`schemaAgentMethods`/`schemaClientMethods`/`servedRequests`/`servedNotifications` against the literal `session_update_fact()` dispatch body in `G:claurst_acp_transport_updates.rs` (parsed by splitting on `fn session_update_fact` / next `fn `) and method literals in `G:claurst_acp_transport.rs` + `G:claurst_acp_transport_io.rs` |
| Live verification tools | Release gate `G:claurst_staged_release_gate_tests.rs` (ignored; `GENT_STAGED_CLAURST=<staged claurst> cargo test --release -p gentd staged_release_gate -- --ignored`, run by `ci.yml` and `release.yml` after staging): launches the staged binary with the real launch plan and a fixed MCP roster, completes Gent's `initialize`/`session/new`, and drives a scripted loopback model through one bound `Write` permission (allowed) and one `mcp__gate__echo` permission (denied). `tools/stage-claurst-runtime.py` refuses any Claurst whose extracted binary differs from `providers.claurst.artifacts[target].binary_sha256` |

## Run lifecycle

- One ACP `session/prompt` is one turn; its `stopReason` is the only terminal authority.
- Start errors are typed: anything that fails before `session/prompt` is written is
  `PortError::NotDelivered`, and the prompt fails with a notice (never unprovable); a failed
  prompt write or a later failure stays unprovable.
- Claurst forwards permission requests for the whole ACP session, not only while a
  `session/prompt` is open. A background `Agent` child (observed with qwen3-1-7b) streams its
  own `tool_call`/`tool_call_update` updates on the parent session; each child `tool_call` and
  `session/request_permission` `toolCall` carries `_meta.gentParentToolCallId` (the launching
  `Agent` tool call), and Claurst always sends a tool's `tool_call` before its permission
  request. Gentd records the child as `ChildStarted` (child id = that tool call id), its tools
  with `parentToolUseId` and its permission with `childId`, so the ledger keeps it past the
  parent's terminal. The conversation policy decides it like any other request; in Ask it is a
  pending decision with no root `waitingPermission`. When the prompt answers, the transport
  answers `cancelled` only the turn's own open requests; a child's stays answerable, also after
  a later prompt starts. `session/cancel` (Stop) answers every open request `cancelled`.
  Claurst reports a child's end only after its loop has returned (a permission wait returns at
  once on cancellation), after the child's own tool ends. When the `Agent` tool ends, Gentd ends
  every child tool still open with the child's phase and records `ChildTerminal`, which settles
  the child's pending decision. A prompt while another is unanswered is refused as
  `PromptInFlight`. No retry or resend exists.
- Permissions follow the saved policy exactly as for Claude (`docs/providers/claude.md`):
  `G:private_claurst_ingress_permission_queue.rs::reconsider_pending_permission` answers a
  pending request the saved policy now decides; sandbox evidence comes from
  `PrivateClaurstBridge::permission_enforcement` for the policy being evaluated.

## If X changes in a new Claurst version…

| Trigger | Edit | Run |
|---|---|---|
| A new `sessionUpdate` kind is emitted | Add a match arm in `G:claurst_acp_transport_updates.rs::session_update_fact()` (never leave it falling to `_ => None` if it carries user-visible content) | `cargo test -p gent-drivers --test provider_contract_drift` |
| `available_commands_update` starts being emitted | Design the `CommandDescriptor` mapping per `docs/provider-commands-and-updates.md` §1, add parsing in `claurst_acp_transport_updates.rs` | drift test (will already be failing with `session-update-new available_commands_update`) |
| A new ACP method (`session/*`) Claurst serves or expects | `G:claurst_acp_transport.rs` / `G:claurst_acp_transport_io.rs` | `cargo test -p gent-drivers --test provider_contract_drift` |
| The release tag or llama.cpp runtime tag bumps | `fixtures/provider-contracts/pins.json` (`providers.claurst.artifacts` and `runtimes.llama_cpp.artifacts`, per target, both must cover the same target set) | `python3 tools/stage-claurst-runtime.py`, `(cd tools && python3 test-provider-pins.py && python3 test-stage-claurst-runtime.py)` |
| The `agent-client-protocol-schema` crate version locked for introspection changes | Recapture with `provider-contract-snapshot.py claurst --claurst-checkout <tag> --acp-schema <new crate dir>` | `crates/gent-drivers/tests/provider_contract_drift.rs` |
| Permission/tool-call correlation shape changes (`tool_call`/`tool_call_update` fields) | `G:claurst_acp_transport_updates.rs::{tool_call(),tool_call_update(),OpenTool,PermissionTool}` | `cargo test -p gentd claurst_` |

## Notes for a future cleanup (report only, not applied here)

- Claurst has no per-provider `README.md` yet, unlike the aspiration in
  `docs/provider-update-playbook.md` §2 — this file is that map until the module-split lands.
- The playbook's §1 table describes Claurst's snapshot as purely "static … checkout" with "none;
  Claurst is never run" for native verification; that undersells it — the shipped runtime *is* a
  sha256-pinned release binary (plus a second sha256-pinned llama.cpp binary), verified at stage
  time by `tools/stage-claurst-runtime.py`. Only the *protocol* snapshot is checkout-based.

## Context compaction (Gent-owned)

Claurst's ACP has no compact method, and a local model's 32k window is small, so Gentd owns
compaction for Claurst. Committed facts stay immutable (no-snapshot contract): compaction appends
a new fact, never rewrites or deletes history, and the rendered provider input stays a disposable
derivation. The summary is a durable fact, not a cache, because it is model output that cannot be
re-derived deterministically.

**Before (verified 2026-09-15).** A run's ACP session is reused for later prompts while the runtime
identity is unchanged (`run_sessions`); any fresh session (first prompt of a run, daemon restart,
selection/permission/MCP change, failure, interrupt) renders ledger history through
`render_fresh_conversation_input` within `history_input_bytes` (Qwen3 1.7B, low effort:
(32768 − 2400 − 2048 − 1024) tokens × 3 = 81,888 bytes). Over budget, the oldest turns were
dropped behind "Earlier history was omitted" with no user-visible notice.

**Fact.** One `events` row per turn, kind `agentChatContextCompaction`, event id
`context-compaction:<turnId>`, committed in the same transaction as its transcript notice:

```
{"type":"compacted","conversationId","runId","turnId","trigger":"command|budget",
 "coversThroughOrdinal":12,"coveredDigestSha256":"…","importsCovered":true,
 "summary":"… ≤16 KiB","tokenEstimate":812,"omittedSourceItems":0}
{"type":"failed","conversationId","runId","turnId","trigger","attemptedThroughOrdinal":12,
 "failure":"runtimeUnavailable|outputLimit|emptySummary"}
```

`coveredDigestSha256` is SHA-256 over `gent-context-compaction-v1`, whether imported history is
covered, and each covered entry's `(ordinal, textDigest)`. A later projection uses the newest
`compacted` fact whose digest equals the digest of the entries *that run* sees through
`coversThroughOrdinal` (run context policy and inherited ordinal applied). A switch with preserved
context therefore reuses the summary, a cleared run never does, and no lineage rules are needed.

**Seeding.** `FrozenConversationContext` gains `summary: {coversThroughOrdinal, importsCovered, text}` and
`earlierHistoryOmitted` (the projection itself left entries or transcript items out). The
renderer emits `[Summary of earlier conversation · tag]` first, then only entries after the
coverage point; the tag digest includes the summary, so message text cannot forge it. The window is
reported as `complete`, `summarized` or `truncated` (anything omitted). Claurst always uses a valid
summary. Decision for cross-provider seeding: Claude and Codex do not read Gent summaries. Their
fresh sessions (switch, recovery) keep rendering full ledger history within their 48/64 KiB replay
bound with the explicit omission marker, and native compaction governs their live sessions. Using
the summary there needs the compaction ledger in ~40 generic Claude/Codex lifecycle bounds, which
is a separate change; until then a Claurst summary is ignored on those paths, never mixed in. The
bridge continues a run's ACP session only while the request's summary coverage equals the
session's seed and the window is not `truncated`; otherwise it starts a fresh session.

**Summarization.** Gentd posts to the running runtime's llama-server
`/v1/chat/completions` (the conversation's shared server; no second server, no ACP session, no Claurst
tools): no `tools` field, `chat_template_kwargs {gent_instructions: "", enable_thinking: false}`,
`temperature 0.2`, `max_tokens 1024`, 300 s timeout. The request asks for bulleted notes that keep
previous notes and copy facts and codes exactly; Qwen3 1.7B echoed the transcript for a prose
"summary" prompt but preserved codewords with this one, including through a rolling update. It is not a turn: nothing is written to the
transcript except the notice. Input per request ≤ (context_tokens − 1024 − 512) × 3 bytes; items are
truncated to 4 KiB each; history is rolled in chunks, `summary_k = summarize(summary_{k−1},
chunk_k)`, at most 4 requests per compaction. Older source beyond 4 chunks is left out and counted
in `omittedSourceItems` (typed notice, never silent). Success requires `finish_reason: stop` and a
non-empty ≤ 16 KiB text after stripping `<think>`; otherwise `outputLimit`/`emptySummary`;
transport errors are `runtimeUnavailable`.

**`/compact`.** Catalog row: Gent intent `compact`, no arguments, `blockedWhileTurnActive`, needs
earlier history (`commandRequiresProviderSession` otherwise). It resolves to a receipt-backed
`SendPrompt "/compact"` exactly like Codex, so receipts, queueing behind a running turn and remote
relay are the existing prompt path. The Claurst lifecycle recognises that turn (no attachments),
starts the turn, runs compaction covering everything through the command's own ordinal, records
the fact plus `PROVIDER_CONTEXT_COMPACTED_NOTICE` and settles `completed`; on failure it records
the failed fact plus `PROVIDER_CONTEXT_COMPACTION_FAILED_NOTICE` and settles `failed`. With no new
source since the last summary the previous summary text is re-recorded with the new coverage and
no model call. Interrupt aborts the request (the HTTP drop cancels generation) and settles
`interrupted` without a fact.

**Auto-compaction.** Before starting a Claurst prompt, Gentd projects and renders; if the window
is `truncated` it compacts first inside the same started turn, then starts the prompt from the new
context. Targets give headroom: the kept tail is ≤ 40% of `history_input_bytes` (weighted
`text + 256` per item) and ≤ 64 entries and ≤ 100 transcript items; everything older is covered.
After compaction the render is ≈ 50% of the budget, so the next compaction needs ≈ half a window
of new history — no per-turn storms. If summarization fails, the prompt still runs with today's
bounded truncation plus `CONTEXT_COMPACTION_FALLBACK_NOTICE`; a `failed` fact suppresses retries
until 8 more entries exist beyond `attemptedThroughOrdinal`. A compaction holds its conversation
like a turn. Its requests queue on the shared llama-server slot alongside other conversations'
turns (`docs/conversation-orchestration.md`). Interrupts, permissions and drains keep being served
while the request runs in a spawned task.

**Remote parity.** Notices are transcript facts, so paired devices receive them through the
existing projection relay; `/compact` from a viewer is the same `gentd-intent invokeCommand`.

**Code map.** `gent-types::conversation_context_compaction` (fact, notices, digest);
`gent-ports::ContextCompactionLedger` + `gent-store` (event row + notice, one transaction);
`gent-runtime::conversation_context_compaction{,_source}` (valid-summary projection, tail target,
source);
`gent-drivers::conversation_context_summary` (summary seed render, summarizer requests);
`G:claurst_prompt_lifecycle/compaction.rs` (admission, task, settle), `G:claurst_context_summarizer.rs`
(llama HTTP, behind `ClaurstRuntimeFactory::context_summarizer`);
`gent-core::agent_chat_commands` (catalog row); `runtime_facade_command_intents.rs` (resolve).

**Tests.** Types: fact serde rejects unknown fields; digest ordering. Store: fact+notice atomic,
idempotent retry, newest-first read. Runtime: valid summary chosen by digest; clear run ignores
it; preserved switch reuses it; tail target bytes/entries; rolling source bounds and omitted count.
Drivers: summary seed before tail, unforgeable header, `complete/summarized/truncated`, summary pinned
while the tail is trimmed; summarizer request has no tools and kwargs above. Gentd with
`FakePrivateClaurstBridge` + fake summarizer: `/compact` settles with one notice and no ACP prompt;
the next prompt's ACP input starts with the summary and omits covered turns; auto-compaction at the
threshold then no recompaction on the next prompt; failure falls back with the typed notice and
backoff; interrupt mid-summary; blocked while a turn runs; receipt retry records one fact; committed
transcript rows unchanged. Catalog: core row, facade resolution (the `agent-chat-commands` IPC
fixture is unchanged: the wire shape is the existing `gentIntent compact`), TUI invokes
`/compact` for Claurst, app resolution + owner/viewer notice through the fake mux.

Live (2026-09-15, debug gentd, Claurst 0.1.7, one llama-server, Qwen3 1.7B low effort): codewords, two
turns, `/compact` (2.0 s, one notice) → the recall turn started a fresh ACP session and returned both
codewords from the summary. Eight 11 KiB turns → auto-compaction on the eighth (covered ordinals 1–6,
tail kept, 35.9 s including the prompt, no fallback notice) → both codewords recalled, the next turn
continued the reseeded session, one fact total.
