# Integration gap review

Audited blockers with code evidence. Every row was re-verified against the working tree; do not
trust a row without re-reading its cited file, and correct the row when the code has moved.

Path convention: `crates/…` and `tools/…` are in this repo (`gent-cli`). Paths prefixed `native:` are
in `/Users/ivanmatiasfort/Clouseau/clouseau-app`, relative to its root.

## P0 prerequisites

### 1. Flutter resolves a different data directory than `gent`/`gentd`

Rust side is done. `crates/gent-types/src/paths.rs` is the one resolver — `default_data_dir` (L18),
`local_socket_path` (L34), `windows_pipe_name` (L43), `resolve_sibling_binary` (L63) — exported from
`crates/gent-types/src/lib.rs:130` and used by `crates/gent-cli/src/local_ipc.rs` and `gentd`'s
`startup.rs`. `default_data_dir()` returns `$GENT_DATA_DIR`, else `BaseDirs::home_dir()/.gent-cli`.
It is a home-directory path on every platform, not a platform data directory. `gent data-dir`
(`crates/gent-cli/src/command_model.rs:125`, executed at `command_execution.rs:110`) and
`gentd --print-data-dir` (`crates/gentd/src/daemon_bootstrap.rs:37`, handled at L105 before IPC bind
and host lock) print it for any external host.

`native:app/lib/service/gentd/gentd_app_runtime.dart:39-48` recomputes it: `$GENT_DATA_DIR`, else
`<home>/.gentd`, else `<app-support>/gentd`. A user who has run `gent` and then opens the app gets a
second daemon and a second ledger.

Resolution: delete the computation in `dataDirectory()` and shell out to the bundled
`gentd --print-data-dir` once per process, caching the result. Invoke it with no `--data-dir`
argument — passing one makes it echo the argument rather than resolve the default. Do not change
`.gent-cli` to match Dart or Dart to match `.gent-cli`; either is a second hardcoded copy of the
rule. Relocating the canonical directory is a separate migration decision (see Open decisions).

### 2. `paths.rs` and three other new modules are untracked

`git ls-files crates/gent-types/src/paths.rs` is empty. `paths.rs`, `agent_chat_sessions.rs`,
`automations.rs` and `prompt_templates.rs` are untracked, and `observer_tap.rs` is deleted but
uncommitted. Every "Rust side is done" claim in this package depends on files that exist only in one
working tree.

Resolution: commit them before any other item starts. A clean checkout must compile and pass
`cargo test --workspace`.

### 3. No display catalog exists for any composer vocabulary

`gentd-source-of-truth-contract.md` requires Gentd to publish stable-ID display records for
provider, model, effort, mode, permission policy, tool source, status chip and composer action.
Only one such record exists today: `LocalModel.label` (`crates/gent-protocol/src/local_models.rs:18`).
Grepping `gent-protocol` and `gent-types` for `pub label`, `pub ordering`, `pub available` returns
that single field. Every other vocabulary is a bare Rust enum with no label, ordering, availability
or explanation: `AgentChatProvider`, `AgentChatEffort`, `AgentChatMode`
(`crates/gent-types/src/agent_chat.rs`), `PermissionMode`, `PermissionCategory`
(`crates/gent-types/src/policies.rs`).

A client therefore cannot render any control without hardcoding the vocabulary, which is exactly
what the contract forbids. This — not the mode/permission split — is the substance of backlog
item 2.

Resolution: add one catalog capability publishing a single generic record type reused for every
vocabulary. Record fields: `id`, `label`, `ordering`, `available`, `unavailable_reason`,
`explanation`, `requires_confirmation`, `scope`. Catalogs are keyed by a catalog ID
(`provider`, `model`, `effort`, `mode`, `permission-policy`, `tool-source`, `composer-action`) so a
new catalog needs no new frame type.

### 4. Transcript and activity facts carry identity but no content

Identity and ordering are already sufficient. `NormalizedTranscriptEvent`
(`crates/gent-types/src/agent_chat.rs:148`) carries `cursor`, `event_id`, `turn_id`, `run_id`,
`kind`, `text`, `is_partial`. `ConversationActivityFact`
(`crates/gent-types/src/conversation_activity.rs:32`) already carries a flattened
`ConversationActivityScope` (`conversation_id`, `run_id`, `turn_id`, `host_epoch`, `cursor`) on every
variant, and the variants already cover `ContextUsage`, `WorkPhase{work_id, kind}`,
`SubagentStarted{child_id, parent_tool_use_id}`, `DecisionPending`/`DecisionSettled`,
`InterruptRequested`, `Recovered` and `Terminal`.

What is missing is content. `ToolActivity` (`crates/gent-types/src/tool_activity.rs:30`) is
deliberately content-free: `tool_use_id`, `tool_name`, `phase`, `output_digest`. There is no tool
input, no tool output body, no diff, no content block, no command output, no child task text or
child model. Native `ToolUseInfo` and `ChildAgentRecord` need all of it.

Resolution: add a work-item read keyed by `tool_use_id` / `work_id` / `child_id` returning bounded,
paged content — tool input JSON, output body, diff, content blocks, command output, child task and
child model — correlated to the existing `ConversationActivityFact` cursors. Do not widen
`ToolActivity` itself; it is on the hot activity stream and must stay small.

### 5. The app drops `gent` from the release archive it already downloads

The Gent release archive already contains everything. `tools/package-release.py:38` packages `gent`
and `gentd` (plus `gent-launcher.exe` on Windows); L164 lays out:

```
gent-{version}-{target}/gent
gent-{version}-{target}/gentd
gent-{version}-{target}/runtime/node/bin/{node,npm}
gent-{version}-{target}/runtime/node/lib/node_modules/npm/bin/npm-cli.js
gent-{version}-{target}/runtime/claurst/claurst
gent-{version}-{target}/runtime/claurst/llama/llama-server
```

llama.cpp is in that archive, under `runtime/claurst/llama/`
(`tools/stage-claurst-runtime.py:94-119`, required by `package-release.py:70-74`). Any plan text
saying llama.cpp still needs packaging beside Gentd is wrong.

The app then throws most of it away. `native:tools/stage-gentd.py:129-133` keeps only the `gentd` binary and
`runtime/`, dropping `gent` and `gent-launcher.exe`; L141-148 then overwrites the archive's
`runtime/node` and `runtime/claurst` with the app's own separately downloaded copies. `native:mate.json`
`bundled` pins `nodejs`, `claurst`, `llama_cpp` and `gentd` as four independently versioned
components with no `gent` entry. `native:app/lib/util/bundled_runtime.dart` resolves `node`/`npm`/`npx`
(L13), `gentd` (L71) and `claurst` (L85) — never `gent`.

Resolution: delete the filter at `native:tools/stage-gentd.py:129-133` and the runtime overwrite at L141-148;
stage the archive intact. Collapse `native:mate.json`'s four `bundled` entries into one `gentRuntime`
entry (version, repo, per-target sha256). Add a `resolveGent()` to `BundledRuntime` and install
rules for `gent` in `native:app/macos/Scripts/bundle_claurst.sh`, `native:app/windows/CMakeLists.txt:127-135` and
`native:app/linux/CMakeLists.txt:159-168`.

### 6. Desktop target matrix is short two architectures, not one

`.github/workflows/release.yml:89-124` in this repo builds four targets:
`x86_64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`,
`x86_64-pc-windows-msvc`. There is no `aarch64-unknown-linux-gnu` and no
`aarch64-pc-windows-msvc`.

Linux ARM64 is the asymmetric case: `tools/stage-claurst-runtime.py:27-30` has a working
`aarch64-unknown-linux-gnu` entry and the app's `native:mate.json` already pins `linux_aarch64` shas, but
this repo never builds a Linux ARM64 archive, so `native:tools/stage-gentd.py:13-17` has nothing to stage.
Consuming one intact Gent archive (item 5) therefore *regresses* Linux ARM64 unless the release
matrix adds it first.

Windows ARM64 does not exist anywhere: no release target, no `stage-claurst-runtime.py` entry, no
`native:mate.json` sha key, and `native:app/windows/Scripts/download_claurst.ps1:16-22` throws
`"Windows ARM64 Claurst runtime is not published"` — an assertion pinned by
`native:tools/test-claurst-download-scripts.py:40`.

The app also stages only one macOS gentd: `native:.github/workflows/release.yml:241` passes
`--target aarch64-apple-darwin` even though this repo publishes an `x86_64-apple-darwin` archive.

Resolution, in order: add `aarch64-unknown-linux-gnu` to this repo's release matrix; publish or
build a Windows ARM64 Claurst/llama.cpp runtime and add `aarch64-pc-windows-msvc`; then update
`download_claurst.ps1` and its test, and the app's release workflow to stage both macOS targets.

### 7. Mobile companions cannot reach a local IPC endpoint

Android and iOS cannot open a Unix socket or named pipe on a desktop. Native mobile Agent Chat runs
through `native:app/lib/provider/agent_chat/agent_chat_remote.dart` and the existing mux transport.

Resolution: the desktop host proxies Gentd snapshots, deltas and intents over the existing
authenticated mux transport as an opaque typed passthrough. Companions never launch Gentd and never
see a local endpoint path.

## P1 implementation gaps

Detailed in `integration-gap-review-p1.md`:

- `provider-auth-v1` is defined but never advertised
- Flutter re-spawned `gentd` indefinitely once the daemon was externally owned — FIXED
- `agent-chat-projection-v1` does not exist and must replace the granular capabilities
- The release manifest's capability list is already stale
- The FRB bridge hand-copies the pipe hash, and its only test asserts a wrong value
- `standalone_mcp_config.rs` duplicates sibling-binary resolution
- Fork, resume and checkpoint have no public intents
- Side questions (`/btw`) have no Gentd equivalent — decided: keep, new capability specified
- MCP authority is startup-config-shaped

## P2 completeness work

- Expand client DTOs for effort availability, recap, preview, workspace, Git, MCP, attention, unread
  and summary metadata.
- Define cross-conversation stream behavior for title/recap changes, search pagination, attention
  acknowledgement, rename/archive/delete, draft ownership and concurrent terminal/native edits.
- Give the app updater sole ownership of the signed bundled `gent`/`gentd`. `runtime-update-check-v1`
  and `runtime-maintenance-v1` already exist (`crates/gent-runtime/src/catalog.rs`); they must report
  an available update inside an installed app bundle and refuse to mutate it.
- Define descriptor evolution rules: stable IDs, display order, availability, generic rendering of
  unknown records, action capability negotiation. A new catalog entry must not require a Flutter
  release.
- Audit every native control against `native-surface-disposition.md`.

## Decisions the user made this pass

**Canonical data directory: `<home>/.gentd`, not `<home>/.gent-cli`, and not a platform-native
Application-Support/AppData path.** DONE. `crates/gent-types/src/paths.rs` (`DATA_DIR_NAME`) and
every doc/script reference renamed. A one-time, one-directional migration —
`gent_types::migrate_legacy_default_data_dir()`, called from the top of
`crates/gentd/src/daemon_bootstrap.rs::run()` whenever `--data-dir`/`GENT_DATA_DIR` was not given —
renames an existing `<home>/.gent-cli` into `<home>/.gentd` the first time a post-rename `gentd`
starts, so existing users' conversation history is not lost. This is not a compatibility shim: it
runs once, is a no-op the moment `.gentd` exists, and no code anywhere reads `.gent-cli` afterward.
Covered by `crates/gent-types/src/paths.rs`'s `migration_*` tests.

**App-owned daemon shutdown: gentd shuts itself down on inactivity, not when any one client
quits.** The user's reasoning: if neither `gent` nor the app has any reason to need it, why keep it
running — which is a stronger, better policy than the "did the spawning app quit" question this
review originally posed, because it also covers `gent` disconnecting last, and it makes the
app-owned-vs-externally-owned distinction irrelevant to shutdown (that distinction, tracked in
`GentdAppRuntime`, still matters for *relaunch* — see the fixed item in `integration-gap-review-p1.md` — just not for *exit*).

Decided shape, not yet built (this needs its own review pass, not a blind implementation):
- gentd self-exits after a grace period (a candidate default is 30s; tunable) with no active client
  presence, releasing `host_lock` cleanly on the way out.
- "Presence" cannot be "an open connection", because `native:app/lib/service/gentd/gentd_ipc_client.dart`'s
  `_open()` opens a fresh transport per RPC and closes it when the call completes (see "The FRB
  bridge hand-copies the pipe hash" in `integration-gap-review-p1.md` for the same transport) — most of an idle-but-open app's
  time has zero open connections, so that signal would fire the shutdown constantly while the app is
  simply not mid-message. Presence must instead combine: (a) a rolling last-activity timestamp
  updated on every accepted request, so ordinary RPC traffic keeps resetting the grace timer, and
  (b) any currently-open long-lived subscription (`event-stream-v1`, `agent-chat-turn-follow-v1`)
  counting as presence for its entire open duration regardless of message rate.
- Needs a concrete answer, during implementation, to whether the native app keeps any subscription
  open continuously while its window is open-but-idle (not actively viewing agent chat). If not, the
  app needs a lightweight always-open presence subscription for this to work — that is new native
  code, not just a gentd change.
- A remote companion reaching the desktop through the proxy (item 7 in the backlog) counts as
  presence through the same desktop connection it proxies over; it must not need its own signal.
- No client — neither `gent` nor the native app — needs to send an explicit shutdown or kill any
  process. This eliminates the ownership-tracking-for-shutdown code that would otherwise be needed on
  both clients.

Resolved: `/btw` side questions survive, as a new Gentd capability. See "Side questions (`/btw`) have
no Gentd equivalent" in `integration-gap-review-p1.md` for the specified shape; the implementation is scoped into
`implementation-backlog.md` item 5.

## Implementation order

Sequenced and expanded as items 0-9 of `implementation-backlog.md`. That file is the single ordering;
this review supplies its evidence.
