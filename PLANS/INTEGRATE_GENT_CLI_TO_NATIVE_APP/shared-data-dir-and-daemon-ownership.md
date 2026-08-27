# Shared data directory and daemon ownership

Record of backlog item 1's Rust pass: what was built, and what it left for the Flutter side. It is
not a second source of truth — `integration-gap-review.md` carries the current evidence and
`implementation-backlog.md` carries the current work. Where they disagree with this file, they win.

`native:` paths are in `/Users/ivanmatiasfort/Clouseau/clouseau-app`.

## Built: one resolver, `gent_types::paths`

`crates/gent-types/src/paths.rs` is the single implementation of:

- `default_data_dir()` — `$GENT_DATA_DIR` if set and non-empty, else
  `directories::BaseDirs::home_dir().join(".gentd")`. This is a home-directory path on every
  platform, not a platform data directory (that was a genuinely open decision; the user decided to
  keep a home-directory path, just renamed from the original `.gent-cli` — see below). Previously
  duplicated six times: `gent-cli`'s
  `local_ipc.rs`, `conversation_activity.rs`, `conversation_timeline.rs`, `conversation_status.rs`,
  and `gentd`'s `startup.rs`, each with its own `BaseDirs` fallback.
- `local_socket_path(data_dir)` — `<data_dir>/gentd.sock`.
- `windows_pipe_name(data_dir)` — `\\.\pipe\gentd-{16-hex FNV-1a}`. Previously duplicated in
  `gent-cli/local_ipc.rs` and `gentd/daemon_bootstrap.rs`.
- `resolve_sibling_binary(name)` — walks up from `current_exe()` for a same-tree sibling binary,
  generalizing what was `default_daemon_binary` in `local_ipc.rs`.

Exported from `crates/gent-types/src/lib.rs:130`. It went into the existing shared-values crate
rather than a new one, since `gent-cli` and `gentd` both already depended on it.

Two callers still wrap or duplicate it rather than calling it:
`crates/gent-cli/src/local_ipc.rs:206 default_daemon_binary` is a thin wrapper (harmless, but it is
the last indirection), and `crates/gentd/src/standalone_mcp_config.rs:266 gent_cli_executable()`
open-codes a shallower two-level walk for `gent` instead of calling `resolve_sibling_binary`. Backlog
item 1.4 removes the second.

**These files are untracked.** `paths.rs`, `agent_chat_sessions.rs`, `automations.rs` and
`prompt_templates.rs` do not appear in `git ls-files`, and `observer_tap.rs` is deleted but
uncommitted. Committing them is backlog item 0.

## Built: the cross-repo bridge

- `gent data-dir` (`crates/gent-cli/src/command_model.rs:125`, executed at
  `command_execution.rs:110`) prints the resolved directory and exits 0 without contacting or
  starting a daemon. Asserted by
  `crates/gent-cli/tests/cli_ipc.rs:201 cli_prints_its_resolved_data_dir_without_contacting_or_starting_a_daemon`,
  which checks no `gentd.sock` appears as a side effect.
- `gentd --print-data-dir` (`crates/gentd/src/daemon_bootstrap.rs:37`, handled at L105) does the same
  on the daemon binary, which is the one a native host has staged. Asserted by
  `crates/gentd/tests/daemon_ipc.rs:36 daemon_prints_its_resolved_data_dir_without_binding_ipc_or_the_host_lock`,
  which checks neither `gentd.sock` nor `gentd.lock` appears.

Either binary is the machine-readable answer to "where does Gent's data live". This is deliberately a
subprocess call rather than a wire frame: resolution must work before any daemon exists, so it cannot
go through the handshake. Invoke it with no `--data-dir` argument, or it echoes the argument.

## Built: the daemon lock names its owner

`crates/gentd/src/host_lock.rs` writes the owning process's pid and `CARGO_PKG_VERSION` as two plain
lines into `<data_dir>/gentd.lock` after the exclusive `flock` succeeds. A conflicting `acquire()`
reads it best-effort and raises
`"gentd pid <pid> (version <version>) already owns <dir>: <os error>"` (L63). A lock file with no
owner metadata falls back to the generic message; the format is self-describing, so there is nothing
to migrate.

This does not change behavior — the exclusive `flock` already allowed only one `gentd` per directory.
It makes the failure diagnosable from stderr alone, on any platform, without a protocol change.
`native:app/lib/service/gentd/gentd_app_runtime.dart:91-92` currently drains and discards that stderr.

## Resolved: the handshake extension is safe to add

This pass deferred backlog item 1.5 (adding daemon build/capability fields to `Negotiated` and
`HostStatus`) because the Dart client owns an independent frame decoder that this repo cannot test,
and it was unknown whether that decoder rejects unknown JSON keys.

It does not. `GentdIpcClient._object` casts to `Map<String, Object?>` with no key validation, every
`fromJson` reads named keys only, and `_open` reads just `protocol` and `capabilities` from the
negotiation body. Adding `daemon_version: String` to `Negotiated`
(`crates/gent-protocol/src/lib.rs:124`) and `HostStatus` (`crates/gent-types/src/lib.rs:256`) is
additive and safe. The deferral no longer applies; the work is backlog item 1.5.

## Flutter defects found, and fixed in a later pass

1. **`GentdAppRuntime.dataDirectory()`** used to default to its own `<home>/.gentd`/app-support
   computation instead of calling `gentd --print-data-dir`. FIXED: it now calls the bundled `gentd`
   once, caches the result, and computes nothing itself.

2. **`GentdAppRuntime._launch()`** used to always spawn and never try connecting first, causing a
   respawn every `client()` call once a daemon was externally owned (see `integration-gap-review.md`
   for the full trace). FIXED: `client()` now calls `_ensureAvailable`, which tries a connection
   before ever spawning, mirroring `crates/gent-cli/src/local_ipc.rs:113 connect_or_start`. Covered
   by `native:app/test/unit/gentd_app_runtime_test.dart`.

3. **`native:app/rust/src/api/gentd_ipc.rs`** still hand-copies the pipe-name derivation instead of
   depending on `gent-types`. Not yet fixed — deferred because it is a cross-repo Cargo dependency;
   see `implementation-backlog.md` item 1.3 for why and what it needs before it is safe to wire in.

## Also decided and built in that later pass, not part of this pass's original scope

- **Canonical directory renamed `.gent-cli` → `.gentd`**, with a one-time migration
  (`gent_types::migrate_legacy_default_data_dir`, called from `daemon_bootstrap::run` before any
  data directory is created or opened) so an existing user's ledger is moved into place rather than
  orphaned. See `integration-gap-review.md`, "Decisions the user made this pass".

## Still open

- **Health-check and reconnect contract** beyond `wait_for_connection_until`
  (`crates/gent-cli/src/local_ipc.rs:167`), which already retries connect while watching a spawned
  child's exit status. Extend it; do not replace it. On the Flutter side, `reportUnavailable()` now
  exists as the hook for this but nothing calls it yet for a daemon Flutter connected to without
  spawning — see `implementation-backlog.md` item 1, step 2's note.
- **App-owned daemon shutdown policy.** Decided: idle-based self-shutdown, independent of which
  client spawned it or quits. Specified in `integration-gap-review.md`; not yet built —
  `implementation-backlog.md` item 1, step 6.
