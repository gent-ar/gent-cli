# Claude/Codex authority port plan

Maps the native app's proven Claude/Codex drivers into gent-cli's dormant
Rust authority chain (item 4 in `continuation-handoff.md`). Read this before
writing any Rust for item 4. Reference only — never embed/depend on app code
at gent-cli runtime; port protocol *knowledge*, not Dart architecture.

## The chain already exists and is dormant, not missing

Every layer of a real Claude/Codex spawn-and-normalize pipeline is already
implemented and tested. `crates/gentd/src/main.rs` declares each module
`#[allow(dead_code)]`; `daemon_bootstrap.rs` has zero references to any of
them (`grep` confirms). This is a composition gap, not a from-scratch build:

| Layer | Rust file(s) | Dart reference | State |
|---|---|---|---|
| Process spawn + stdin frame | `gent-drivers/src/claude_runner.rs`, `codex_runner.rs` | `claude_driver.dart` spawn args, `encodeUserMessage` | Real, matches app's `{"type":"user","message":{...}}` shape |
| Frame normalization | `gent-drivers/src/public_protocol.rs` (+`codex_protocol.rs`) | `claude_driver.dart::parseStreamEvent`, `codex_driver.dart` | Real but **narrower** than Dart — see gaps below |
| Codex turn state machine | `gent-drivers/src/codex_turn.rs` (not yet read this pass) | `codex_driver.dart` JSON-RPC handling | Present, untraced in this pass |
| Per-provider lifecycle dispatch/poll | `gentd/src/claude_prompt_lifecycle.rs`, `codex_prompt_lifecycle.rs` | n/a (app has no daemon-side dispatch concept) | Real; persist-before-broadcast via `record_wire()` before poll returns |
| Lifecycle state machine | `gentd/src/claude_authority_supervisor.rs`, `codex_authority_supervisor.rs` | n/a | Real: `AwaitingRecovery→Running→ShutdownDraining{Interrupt→Terminate→Kill}→Stopped`, provider-agnostic |
| Authority composition (the seam bootstrap never calls) | `gentd/src/claude_authority_composition.rs`, `codex_authority_composition.rs` | n/a | Real, ~195 lines each, explicitly marked "never selected by daemon bootstrap" |
| Signed evidence gate | `gent-adapters/src/claude_authority_evidence.rs` (+ codex counterpart) | n/a | Real; requires all 15 `ClaudeEvidenceScenario` proofs signed before a profile can compose |
| Runtime wiring surface | `gentd/src/public_driver_runtime.rs` + `_composition.rs` | n/a | Real: `PublicDriversRuntime<L,D,R>`, wraps `PublicRunService`, prompt-dispatch claim/settle, goal resolver injection — "dormant, separately approved... `main` never constructs it" |

`main()` never constructs a `PublicDriversRuntime`, never calls
`compose_private_claude_authority`/`compose_private_codex_authority`, and
`ValidatedAuthorityProfile` never resolves to `PreparedPublicDrivers` in
observer mode. That is the entire remaining gap for *wiring* — the pieces
below are content gaps inside the normalizer, which block wiring from being
useful even once composed.

## Normalizer gaps: `public_protocol.rs` vs `claude_driver.dart`

`claude_driver.dart::parseStreamEvent` (app/lib/model/agent_adapters/driver/claude_driver.dart:417-483)
switches on `json['event']['type']` — i.e. it expects the **`stream_event`
wrapper**, not top-level `type: assistant/system/result` alone. The current
Rust `claude()` reducer (`gent-drivers/src/public_protocol.rs:57-66`) only
handles top-level `system`/`assistant`/`result` and has no `stream_event`
case at all. Concretely missing, all confirmed present and load-bearing in
the Dart driver:

1. **`stream_event` wrapper + nested `event.type`** — `message_start`,
   `content_block_start`, `content_block_delta`, `content_block_stop`,
   `message_stop`, `ping`. Without this the daemon never sees incremental
   text — only the final `assistant` frame's full text block, which is not
   how Claude actually streams under `--output-format stream-json`.
2. **`thinking` content blocks and `thinking_delta`** — `claude_driver.dart:456-458,471-472`.
   No `PublicWireFact`/`NormalizedProviderEvent` variant carries thinking
   text today; `gent-types` needs one before this can be ported (a genuine
   new-type addition, not just a match arm).
3. **`control_request`/`control_response` permission relay** —
   `claude_driver.dart::parseControlRequest` (line 793) and the encode side.
   This is the mechanism verified live for `fixtures/public-driver-transcripts/
   claude-permission-persistent-haiku-20260819.jsonl`: `subtype: can_use_tool`,
   `permission_suggestions` echoed back as `updatedPermissions`. `gentd`
   already has durable permission modes (Default/Plan/Auto-Accept
   Edits/Autonomous/Bypass — see "Current implemented batch"); this is the
   missing wire-level plumbing that would let those modes actually answer a
   live `control_request`. No Rust type for `AgentControlRequest` exists yet.
4. **Sub-agent activity via `parent_tool_use_id`** — `claude_driver.dart:433-437,514-611`.
   The app synthesizes a `subagent_activity` frame from a background
   transcript keyed by `parent_tool_use_id`, decoupled from the main
   `_inToolUse` pairing state so it can interleave safely. `NormalizedProviderEvent`
   already has `ChildStarted{child_id,parent_tool_use_id}`/`ChildTerminal`
   (`gent-types/src/provider_lifecycle_values.rs:68-75`) — the *type* exists,
   nothing in `public_protocol.rs` produces it yet for Claude.
5. **Tool-use streaming deltas** (`input_json_delta`) — accumulates partial
   tool-call JSON across deltas before the tool actually starts executing.
   No Rust equivalent.
6. **`tool_result` frames** (user-role tool output echoed back) — not
   handled by either side of `public_protocol.rs` today; needed to close the
   loop on `ToolActivity{phase: Completed}` with actual output, not just a
   digest placeholder (`output_digest: None` is hardcoded everywhere today).

Compare: the **Codex** normalizer (`gent-drivers/src/public_protocol/codex_protocol.rs`)
is already much closer to parity — it has `item/agentMessage/delta` streaming,
`turn/plan/updated`/`item/plan/delta`, `contextCompaction` item handling,
`requestApproval` → `AttentionRequired`, and `subAgentActivity`/`collabAgentToolCall`
in its `tool_kind()` allowlist. Claude is the side needing the real port work;
Codex mainly needs the same **wiring** (composition into `daemon_bootstrap`),
not new normalizer content, modulo whatever `codex_turn.rs` still lacks
(untraced this pass — read it before starting Codex wiring).

## Suggested order

1. Add the missing `gent-types` vocabulary first (thinking event variant,
   `AgentControlRequest`/`AgentControlResponse` types) — everything downstream
   depends on these existing before a match arm can produce them.
2. Extend `public_protocol.rs`'s `claude()` reducer with the `stream_event`
   wrapper and its nested cases (1, 2, 5 above) under existing test coverage
   patterns (`claude_runner_tests.rs` equivalent) — pure, no daemon wiring yet.
3. Add `control_request`/`control_response` normalization + a pure encode
   path, verified against the real captured fixture
   (`fixtures/public-driver-transcripts/claude-permission-persistent-haiku-20260819.jsonl`)
   rather than a hand-written test frame — this is exactly the kind of claim
   the strict evidence program is designed to keep honest.
4. Add sub-agent correlation (4 above) — lower priority than 1-3; nothing
   downstream blocks on it structurally the way permission relay blocks
   Auto-Accept/Autonomous modes from being real.
5. Only after the normalizer covers the above: wire one real
   `compose_private_claude_authority` call behind a still-unadvertised,
   explicitly-constructed profile (mirroring how `prompt_provider_provision_profile_support.rs`
   proved the provisioning profile end-to-end without touching
   `daemon_bootstrap.rs`) — prove it the same way: real `RuntimeFacade`, real
   wire codec, `tokio::io::duplex`, no daemon-wide composition change.
6. Read `codex_turn.rs` before repeating steps 1-5 for Codex; it may already
   cover more of this list than Claude does.

## What NOT to port

Never port Dart's UI-facing concerns: `AgentAdapter` tool-label formatting,
`ToolCategory`, chip/timeline presentation, or anything in
`agent_chat_tab.dart`/`agent_chat_provider.dart`. Those are native-app-only
and have no gent-cli analog — gent-cli's contract ends at `NormalizedProviderEvent`/
`PublicWireFact`; presentation is a client concern for whichever client
(app or `gent` terminal) renders the stream.
