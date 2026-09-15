# Integration gap review — P1 implementation gaps

Continues `integration-gap-review.md`; the same re-verification rule and path convention apply.

## P1 implementation gaps

### `provider-auth-v1` is defined but never advertised

`crates/gent-protocol/src/provider_auth.rs:15` defines `PROVIDER_AUTH_CAPABILITY`, and
`crates/gentd/src/provider_auth_transport.rs:27` gates its handler on it — but the capability is not
in `DECLARED` (`crates/gent-runtime/src/catalog.rs:44`) nor in the profile-gated additions at L193.
The daemon never announces it, so negotiation can never enable the handler that exists. `gent`'s
`/login` works around this by starting a separate one-off Gentd process.

Resolution: add the capability to the catalog so the composed handler is reachable, then delete the
one-off process path in `/login`. Extend the frame with progress, cancellation, retry and
exactly-once release of the held prompt receipt.

### Flutter re-spawned `gentd` indefinitely once the daemon was externally owned — FIXED

`crates/gent-cli/src/local_ipc.rs:113 connect_or_start` is the proven pattern: try `connect()`
first, spawn only on failure, then `wait_for_connection_until` (L167) polls while watching the
child's exit status. `crates/gentd/src/host_lock.rs:63` makes a lost race legible —
`"gentd pid {pid} (version {version}) already owns {dir}: {error}"`.

`GentdAppRuntime._launch()` used to always spawn and discard stdout/stderr. Traced failure: when a
`gentd` already owned the directory, the spawned child lost the host lock and exited immediately;
its exit handler nulled the cached "daemon is available" future. The readiness poll meanwhile
succeeded against the pre-existing daemon and returned, so the caller never saw an error — but the
cache was already gone, and every subsequent `client()` call spawned another doomed child, for as
long as the daemon stayed externally owned.

Fixed in `native:app/lib/service/gentd/gentd_app_runtime.dart`: `client()` now calls
`_ensureAvailable`, which tries a connection (`_canConnect`, mirroring `connect()`) before ever
calling `_launch`/spawn. `_launch` only runs, and only tracks `_ownedProcess`, when that probe
failed — a daemon found already listening is never tracked as ownable, so its later exit (there is
none, since we never spawned it) can't trigger a respawn. The cached "available" future
(`_ensuring`) is now cleared only by `reportUnavailable()`, called from the owned process's own exit
handler or from a failed attempt — never from an external daemon's activity. `_launch`'s poll loop
now also races the spawned child's `exitCode` (mirroring `wait_for_connection_until`'s
`child.try_wait()`), so a losing host-lock race fails fast with the daemon's captured stderr instead
of silently retrying for the full 15s window. Covered by
`native:app/test/unit/gentd_app_runtime_test.dart` (9 cases, including the exact regression:
repeated `client()` calls after a healthy connection never spawn).

`dataDirectory()` was also fixed in the same change: it now resolves via one cached invocation of
`gentd --print-data-dir` instead of independently computing `<home>/.gentd`, closing the P0 #1 gap
in `integration-gap-review.md` on the Flutter side. Still open from that item: canonical directory relocation (Open decision
1), and the `app/rust`/`standalone_mcp_config.rs` de-duplications below, which were left for a
follow-up change since they touch cross-repo Cargo wiring and mobile build targets that need their
own review.

### `agent-chat-projection-v1` does not exist and must replace the granular capabilities

The name appears only in these plan docs and `docs/continuation-handoff.md`; there is no code.
Meanwhile `native:app/lib/service/gentd/gentd_ipc_client.dart:19-28` already negotiates ten capabilities —
`conversation-index-v1`, `local-models-v1`, `event-stream-v1`, `agent-chat-conversations-v1`,
`agent-chat-intents-v1`, `agent-chat-transcript-v1`, `agent-chat-turn-follow-v1`,
`conversation-activity-v1`, `agent-chat-permissions-v1`, `attachments-v1` — and Gentd declares 27
capability strings in total (`crates/gent-runtime/src/catalog.rs`).

Resolution: `agent-chat-projection-v1` subsumes the six read/follow capabilities
(`conversation-index-v1`, `agent-chat-conversations-v1`, `agent-chat-transcript-v1`,
`agent-chat-turn-follow-v1`, `conversation-activity-v1`, `agent-chat-sessions-v1`) into one
snapshot/delta contract, and those six are deleted in the same change. `agent-chat-intents-v1`,
`agent-chat-permissions-v1`, `attachments-v1`, `local-models-v1` and `permission-policy-v1` stay as
mutation surfaces. Specify DTOs, one total cursor order across transcript/activity/workspace, reset
and paging rules, idempotency receipts, and typed unavailable-capability handling.

### The release manifest's capability list is already stale

`tools/package-release.py:137-144` writes a seven-entry `capabilities` list into each archive's
`manifest.json`, and `native:tools/stage-gentd.py:68-91` verifies it during staging. That list omits
`conversation-index-v1`, `event-stream-v1` and `conversation-activity-v1`, all three of which the
Dart client requires at `gentd_ipc_client.dart:19-28`. The manifest is the intended build-time
compatibility contract and it does not describe the daemon it ships.

Resolution: generate the manifest's `capabilities` from
`gent_runtime::catalog::declared_capabilities_with_profiles()` rather than a hand-written literal, so
it cannot drift.

### The FRB bridge hand-copies the pipe hash, and its only test asserts a wrong value

`native:app/rust/src/api/gentd_ipc.rs:171-178` reimplements `endpoint_hash` inline. It is currently
byte-identical to `gent_types::windows_pipe_name`'s `windows_endpoint_hash`
(`crates/gent-types/src/paths.rs:48`), so there is no live divergence — but nothing enforces that.

Its guard test is broken twice over. `pipe_name_matches_gent_cli_derivation`
(`gentd_ipc.rs:213-220`) is `#[cfg(windows)]` and has evidently never run. Its input is
`Path::new(r"C:\\gent\\data")` — a raw string, so the path literally contains doubled backslashes
rather than the intended `C:\gent\data`. And its expected value matches neither: the doubled form
hashes to `2ef47651e608fcea` and the single-backslash form to `fc7c1d6cff1b182e`, while the test
asserts `8f19425f0de18d88`.

Resolution: add `gent-types` as a path dependency of `native:app/rust` and call
`gent_types::windows_pipe_name` and `local_socket_path` directly. Delete `endpoint_hash`, `pipe_name`
and the broken test — a shared function needs no cross-repo equivalence assertion.

### `standalone_mcp_config.rs` duplicates sibling-binary resolution

`crates/gentd/src/standalone_mcp_config.rs:266 gent_cli_executable()` walks `current_exe()` and its
parent looking for `gent`, duplicating `paths::resolve_sibling_binary` with a shallower two-level
walk. It will fail to find `gent` in bundle layouts that `resolve_sibling_binary` handles.

Resolution: call `gent_types::resolve_sibling_binary("gent")` and delete the local walk.

### Fork, resume and checkpoint have no public intents

Resolution: add create/fork/resume/checkpoint/restore intents to `agent-chat-intents-v1`. Filesystem
restore requires an explicit confirmation receipt distinct from the restore intent itself.

### Side questions (`/btw`) have no Gentd equivalent — decided: keep, new capability specified

Resolved by the user: `/btw` survives. It does not exist in this repo at all — `grep -rl
'side_question\|side-question\|SideQuestion' crates/` is empty — so this is new Gentd scope, not a
port of an existing internal type.

Today it is entirely native-local: `native:app/lib/provider/network/server/controller/agent_chat_btw.dart`
spawns a short-lived helper CLI process per question, bounded by
`native:app/lib/util/side_question_context.dart` (`sideQuestionMessageLimit = 8`,
`sideQuestionCharLimit = 12000`) excerpting the *durable* conversation — deliberately never
resuming/forking the live provider session, because forking re-sends and re-caches the whole
conversation on every ask and makes Claude silently ignore the requested helper model (see that
file's doc comment). It streams `btw_delta`/`btw_begin`/`btw_end` through the same
commit-before-fan-out ledger path as the main turn, enforces per-conversation/global concurrency caps
(3 per conversation, 8 total live) in Dart-side maps (`_btwProcs`, `agent_chat_controller.dart`), and
supports cancellation and a 300s timeout. This is a second, native-only provider-process
implementation — exactly what the source-of-truth contract forbids — and its concurrency caps cannot
be shared across two clients of the same daemon (a terminal and a native app each spawning side
questions against the same Gentd would each enforce the limit independently).

Gentd already has the matching infrastructure shape, but not the on-demand, streaming, cancelable
form this needs: `crates/gentd/src/claude_summary_runner.rs` (and its Codex/Claurst siblings)
implement `gent_ports::conversation_summary::ConversationSummaryRunner` — a bounded-prompt,
bounded-output, timeout-limited helper process launch via `gent_drivers::SystemLauncher` — but it is
internal-only (used by `ConversationSummaryScheduler` for background titles/recaps), takes a fixed
prompt, returns one final `String` with no streaming, and has no cancellation.

Resolution: add a new capability `agent-chat-side-question-v1`.
- Intent `agent-chat.side-question.ask { conversation_id, question, model: Option<String>, request_id
  }`. Gentd builds the bounded transcript excerpt itself from the durable ledger — port
  `recentSideQuestionMessages`/`boundSideQuestionTranscript`'s bounding rule (8 messages, 12000
  chars) into a shared Rust helper so terminal and native derive the identical excerpt, per the
  Remote Parity Rule. Never forks/resumes the live provider session, for the same reason Dart avoids
  it today.
- A new `ConversationSideQuestionRunner` port (or an extended `ConversationSummaryRunner`) that
  streams output chunks instead of returning one `String`, and exposes a cancel handle. Reuse
  `SystemLauncher`/`ProviderLaunch` process-spawn plumbing from the summary runners; do not
  reimplement process management.
- Concurrency limits (3 per conversation, 8 total live) and the 300s timeout move into Gentd, so they
  are enforced once across every client, not per-connected-client.
- `side_question_begin`/`side_question_delta`/`side_question_end` publish as ordinary events on the
  same conversation event stream and cursor order as everything else in
  `agent-chat-projection-v1` (item above) — not a bespoke frame type.
- Cancel intent `agent-chat.side-question.cancel { conversation_id, side_question_id }`.
- Delete `native:app/lib/provider/network/server/controller/agent_chat_btw.dart` and the Dart-side
  concurrency maps in the same cutover that replaces it with calls to the new intents; do not keep
  both paths.

This is durable-product-domain-sized work, scoped into `implementation-backlog.md` item 5 alongside
sessions/templates/automations/MCP.

### MCP authority is startup-config-shaped

`crates/gentd/src/standalone_mcp_config.rs` writes `<data_dir>/standalone-mcp.json` at startup,
registering the internal `gent-automations` and `gent-forge` servers. `forge-connectors-v1` covers
the Forge catalog. Neither gives a client config-source registration, live config updates, connector
health, credential ownership, per-conversation source selection, or reconnect semantics.

Resolution: extend `forge-connectors-v1` to cover all MCP sources rather than adding a parallel
capability, and make the startup JSON one registered source among others rather than the mechanism.
