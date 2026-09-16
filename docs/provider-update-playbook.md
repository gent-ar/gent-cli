# Provider knowledge map and update playbook

Status: phase 1 design, not implemented. It builds on `docs/provider-commands-and-updates.md`
(command catalog, contract snapshots, drift gate). Abbreviations: `D:` is
`crates/gent-drivers/src/`, `G:` is `crates/gentd/src/`, `A:` is `crates/gent-adapters/src/`.

## 1. Where provider protocol knowledge lives today

The verified, per-provider concern-to-file-and-symbol map now lives in `docs/providers/claude.md`,
`docs/providers/codex.md` and `docs/providers/claurst.md` — each also carries an "if X changes,
edit Y, run Z" table. Open the relevant one first; it supersedes the condensed table this section
used to carry. This section stays as the cross-provider summary of what is shared or ambiguous
across all three:

- Tool name → permission category is one mixed-vendor file: `G:permission_category.rs`.
- Native-binary verification (digest/size/mtime, readiness) is shared, not per-provider:
  `G:provider_executables.rs`, `G:standalone_provider_readiness.rs`.
- Provider-upgrade-rebind ledger events (`runExecutableRebound`, `runSessionRetired`,
  `providerSessionRecovered`) are cross-provider primitives in `crates/gent-store` and
  `crates/gent-types`, not owned by any one provider's driver files.
- Pins: Claude and Codex are a signed secret only (`fixtures/provider-contracts/pins.json` is the
  unsigned working copy); Claurst is pinned the same way plus `tools/stage-claurst-runtime.py`.
- Evidence tools: `tools/capture-*.py` and `public_driver_probes.py` cover Claude and Codex (their
  one-shot argv differs from production); Claurst has none — it is never launched by tooling.

Dead or duplicate code: the unused declarative manifest `A:manifest/*` and the two Codex
interrupt paths. Codex has one `initialize` request, `D:message_encoding.rs::codex_initialize_request`,
used by both `encode_codex_handshake` and `D:codex_session.rs`.

## 2. Consolidation

Proportionate means moves and deletions, with no new crate or trait layer.

- One module per provider in `gent-drivers`: `src/claude/`, `src/codex/`, `src/claurst/`.
  Each holds all wire knowledge for that provider: `launch.rs` (every argv, including auth and
  summary variants), `contract.rs` (declared flags, method enums, handled/ignored sets, read
  fields, built-in command dispositions), `protocol/` (frame to fact), `control.rs`
  (permissions, unknown-request replies), `catalog.rs` (initialize or `model/list` parsing for
  models and commands), `errors.rs`, `tools.rs` (tool name to permission category), and
  `README.md`.
- `gentd` keeps lifecycle, ownership and ledger composition (`*_prompt_lifecycle`, runtime
  owners, probes as process I/O). It calls `gent_drivers::<provider>::…` for anything
  wire-shaped. Move the Claurst ACP wire files (`claurst_acp_transport*.rs`, the settings JSON
  and argv builders) to `gent-drivers/src/claurst/`. The allowed dependency graph already
  permits this, and no Claurst credential or endpoint moves.
- Delete: the SIGINT Codex interrupt path (keep `turn/interrupt`), `A:manifest/*`, and the
  per-vendor tool names in `G:permission_category.rs` (split into each `tools.rs`). The Python capture tools read argv
  from `cargo run -p gent-drivers --example provider-contract -- argv <provider>`, so tools and
  production share one argv.
- Enforce the boundary with a `tools/check-architecture.py` rule: in production sources outside
  `gent-drivers/src/{claude,codex,claurst}/`, fail on wire literals such as `"--` flag strings
  for provider executables, JSON-RPC method strings (`"thread/`, `"turn/`, `"item/`,
  `"session/`) and Claude `"control_request"`. Test sources and fake harnesses are exempt.
- Each `README.md` stays under 80 lines and is the map an agent opens first: the pinned
  version and snapshot path, a concern-to-file table, the transcript cells and scenario owners,
  and the live smoke commands.
- Files at the size limit (`claude_runner.rs`, `claurst_acp_transport.rs`,
  `codex_prompt_runner.rs`, `codex_session/types.rs`, `public_protocol.rs`,
  `standalone_mcp_config.rs`) are split by concern during the move, not after.

With that in place, a version bump touches `src/<provider>/` and
`fixtures/provider-contracts/`. When Gent-owned behaviour really changes, it may also touch the
provider's `gentd` lifecycle directory.

## Executed binary trust boundary

- **Install:** Gentd installs exactly the signed platform tarball.
  - `npm pack`, SRI check, then `npm install --offline <tgz>`, so npm cannot fetch anything else.
  - The native binary is not reachable through a shim or `optionalDependencies`.
  - Codex's platform tarball also ships `codex-code-mode-host`, `codex-path/rg` and
    `codex-resources/zsh`. Codex spawns these itself. They are covered only by the install-time
    tarball integrity, not re-verified per launch.
- **Provision:** the executed binary's `--version` and sha256 must match a signed compatibility
  entry.
- **Readiness (every prompt release) and every provider start/resume, including summary launches** (`ProviderExecutables`):
  - The durable provisioned lock must still match the file's size and mtime. The binary is hashed
    only when they changed, and the new hash must still be authorized below.
  - The lock path must be the platform layout.
  - A freshly reloaded signed release must authorize the provider and the lock's
    `(id, version, digest)`.
  - A failure is a typed `InvalidInstallation` readiness review that offers a reinstall. The
    prompt stays held; this is not a turn failure. A reinstall is picked up by the running
    daemon without a restart.
  - A binary altered after the readiness check but before the spawn is refused by the resolver
    and the launcher's digest recheck, so no process starts; that prompt is not held and waits
    for the next wake.
- **Upgrade of an existing conversation** (a newly authorized version or digest is installed):
  - The next start or resume rebinds the run to the verified executable in the same transaction
    that claims its lease, and records a `runExecutableRebound` event with both locks. An idle
    live process launched from the previous binary is released first. The run, its prompts and
    its receipts keep their identity, so a held or pending prompt runs exactly once.
  - The provider session is resumed on the new binary: Claude `--resume` (verified live from
    2.1.233 to 2.1.270) and Codex `thread/resume`.
  - If the new binary cannot find the session, Claude recreates it under the same id from Gent's
    saved history, and Codex starts one fresh thread seeded from that history (the old binding is
    retired with a `runSessionRetired` event). Both record the typed
    `providerSessionRecovered` notice. A Codex thread lost without an upgrade still fails with
    `providerSessionUnavailable` and offers "Continue from saved history".
  - The model catalog relists a provider as soon as its verified executable changes, not only
    after the TTL.
- **Explicit `--standalone-<provider>-executable` developer paths** bypass signed verification
  only in development (debug) builds of `gentd`. A release build refuses to start with them.
- **Codex environment:** Gentd runs `vendor/<triple>/bin/codex` directly.
  - It strips inherited `CODEX_MANAGED_BY_*` / `CODEX_MANAGED_PACKAGE_ROOT`. The binary uses
    them only for doctor and update-target provenance.
  - It launches `app-server -c check_for_update_on_startup=false`, because Gent pins versions.

## 3. Playbook: update to a new Claude Code, Codex or Claurst version

An AI agent follows this playbook end to end. Rules: one provider per run; cargo with `-j 4` and
`-p <crate>` only; cheap models only (`haiku`, `gpt-5.6-luna`); one gentd; never sign or push.
Stop and ask the maintainer at the steps marked HUMAN. `W` is `release-work/<provider>-NEW` and
`EXE` the scratch native binary. A Claude patch release with no contract change took 14 minutes.

1. **Read the map (1 min).** Note `OLD` from `fixtures/provider-contracts/pins.json`. Read
   `docs/providers/<provider>.md` — the concern-to-file table with verified paths and symbols for
   this provider, plus its own "if X changes" table. §1 above stays as the cross-provider
   overview and dead/duplicate-code inventory; the per-provider docs are the map to open first.
2. **Obtain the candidate into scratch, never over the user's install (15 s).** Use any npm.
   - Claude: `npm pack @anthropic-ai/claude-code-<platform>@NEW` in `W`, then
     `npm install --global --prefix W/prefix --ignore-scripts --offline W/<tgz>`.
     `EXE` is `W/prefix/lib/node_modules/@anthropic-ai/claude-code-<platform>/claude`.
   - Codex: the same with `@openai/codex@NEW-<platform>`, never the `@openai/codex` shim.
     `EXE` is `W/prefix/lib/node_modules/@openai/codex/vendor/<triple>/bin/codex`.
   - Claurst: download the release tag archive and record its sha256 per target.
   - Check `EXE --version`.
3. **Snapshot (2 s).** `python3 tools/provider-contract-snapshot.py <provider> --executable EXE`.
   It sends only `initialize`, never a prompt, and writes `fixtures/provider-contracts/<provider>/NEW/`.
   For Claude it also records `frames.json`, the frame and control subtypes named in the
   binary's bundled source. For Claurst, run `... claurst --claurst-checkout <checkout at the
   tag> --acp-schema <locked agent-client-protocol-schema crate dir>`. Never launch Claurst or
   llama-server. If `OLD` has no `frames.json`, recapture `OLD` from its binary first.
4. **Diff (1 s, plus about 1 min per finding).**
   `python3 tools/provider-contract-snapshot.py --diff <provider>/OLD <provider>/NEW`.
   `no contract changes` (exit 0) means skip to step 5 and run only the fast gates. Otherwise
   each line is one finding:
   - `flag-removed` / `flag-shape-changed` (a used flag) → `D:launch_spec.rs` or
     `D:claude_turn_options.rs`, plus the argv unit test.
   - `method-removed` (sent or handled) → delete or replace the branch in
     `D:public_protocol/<provider>_protocol*`, `D:codex_turn*` or `D:codex_session/`, and delete
     tests that only exercised it.
   - `notification-new` → a parser and unit test, or the ignored arm, with one line of
     reasoning in the commit message.
   - `server-request-new` → `D:codex_control.rs`, or confirm the fail-closed `-32601` reply
     covers it. If it gates approvals or auth: HUMAN.
   - `field-changed` / `field-removed` → the parser; build the test frame from the snapshot's
     property tree.
   - `frame-subtype-new` / `frame-subtype-removed` (Claude) → read its context with
     `strings -n 4 EXE | grep -o '.\{160\}<subtype>.\{160\}' | head -3`. A system frame Gent's
     argv or `initialize` can trigger → the `claude()` match in `D:public_protocol.rs`. A
     control request Claude sends → `D:claude_control.rs`. Opt-in, remote-control or cloud-worker
     only → no change; record why in the report.
   - `command-builtin-new` / `-removed` / `-changed` → a disposition row in
     `D:claude_commands.rs`, following §1 of the commands doc. Anything that mutates model, MCP,
     permissions or session identity is `unsupported` or Gent-owned.
   - `models-shape-changed` → `G:model_catalog_claude.rs` or `G:model_catalog_codex.rs`.
   - `stderr-warning` in `launch-probe.json` → fix the argv. Never ignore it.
5. **Pin (1 min).** In `pins.json`, set the version and `--version` string, and per target the
   package version, integrity and native digest printed by
   `python3 tools/provider-native-digest.py <provider> <target> W/<tgz>` (`--package-name` and
   `--executable` for a target not yet pinned). Then rerun it with `--check`. Add a
   `reconciliation.required` line naming the new compatibility entry
   (`<provider>-<version>-<target>`) for owner re-signing. Keep the `OLD` snapshot.
6. **Gate (1 min without Rust changes).** In order, fixing until green:
   `cargo test -j 4 -p gent-drivers --test provider_contract_drift`, `cargo test -j 4 -p gent-drivers`,
   `cargo run --quiet -p gent-testkit --bin validate-public-driver-manifest -- fixtures/public-driver-transcripts/manifest.yml`,
   `(cd tools && python3 test-provider-pins.py && python3 test-provider-contract-snapshot.py)`
   and `python3 tools/check-architecture.py`. Only when Rust changed, also run
   `cargo test -j 4 -p gentd -- <provider>_ installed_provider provider_executables` (about
   2 min). The gentd tests use fake CLIs, so they prove nothing about the new binary.
7. **Live parsing check (30 s).** Prepend a directory holding a `<provider>` symlink to `EXE`
   to `PATH`, then run `python3 tools/verify-live-driver-parsing.py <provider> --model haiku`
   (Codex: `--model gpt-5.6-luna`). The tool resolves the binary from `PATH`, defaults to an
   expensive model and uses the one-shot argv. Compare any `UNRECOGNIZED` line with the same
   run on `OLD` before treating it as drift. Transcript cells record no provider version, and
   `update-public-driver-transcripts.py --run --confirm` refreshes only unrecorded cells. Recapture a
   recorded cell (HUMAN-attended) only when the diff changed that scenario's surface.
8. **Live Gent smoke (7 min).** Build `gentd` and `gent` in debug. In a debug build, run
   `gentd --standalone-authority --standalone-<provider>-executable EXE --data-dir W/data`
   in the background; this needs no signed release. With `gent --data-dir W/data --no-autostart`:
   - `chat create --workspace W/ws --provider <provider> --model haiku --effort low --mode agent`,
     then `chat send` a one-word reply.
   - Steer: `chat send` a long list in the background, then `chat queue` and `chat steer`. If
     the model finishes first, the steer correctly runs once as its own turn.
   - Interrupt: `chat send` a long story in the background, then `chat interrupt --run-id`.
   - Approval: ask for a mutating command (`touch approved.txt`; read-only commands such as
     `pwd` are auto-allowed), then `permissions pending` and `permissions respond
     --decision-id <.binding.decisionId> --decision approve-once`.
   - Resume: stop and restart gentd, then ask about the first message. Only a restart forces
     `--resume`; later prompts reuse the live process. Confirm with
     `ps -axo command | grep -- --resume`.
   - `chat send --text /nope` must fail with `slashCommandRequiresInvoke` and create no turn.
     Once `gent` exposes `InvokeCommand`, also run `/compact` and one `providerNative` command.
   - Check `chat transcript` and `sqlite3 "file:W/data/gent.db?mode=ro" 'select phase from turns'`.
9. **Evidence and signing.** The signed ordinary authority is built in CI, not by hand: the
   `node-runtime` release job measures the Node binary each target ships and the `authority` job
   runs `tools/build-ordinary-authority.py`, which derives the compatibility, package-policy and
   evidence documents from `pins.json`, those measurements and the recorded transcript corpus,
   and signs them with nested keys derived from the runtime-release root key. To rehearse it
   locally, generate a throwaway root with `tools/generate-runtime-release-key.py`, write a
   temporary `root-keys.json`, run the builder with `--root-keys`, and check the result with
   `tools/verify-staged-authority.py` against a locally built `gentd`.
10. **Clean up.** Stop the daemon. Delete `W` and every `~/.claude/projects/<W path with / and .
    as ->` folder the smoke created. Delete snapshots older than `OLD`. Report the diff findings
    with dispositions, the gates run, and any file changed outside `fixtures/` with its reason.

## 4. Implementation plan

Each step ships alone, keeps the tree green, and names the tests it adds.

1. **Pins in the repository.** Add `fixtures/provider-contracts/pins.json` with the shipped
   values. HUMAN supplies them from the decoded release payload. `stage-claurst-runtime.py`
   reads its pin from this file, and the signer refuses mismatches. Tests: signer cases in
   `tools/test-sign-ordinary-authority-release.py` (version mismatch, missing snapshot, digest
   mismatch) and pin loading in `tools/test-stage-claurst-runtime.py`.
2. **Snapshot tool.** Add `tools/provider-contract-snapshot.py` with capture and `--diff`, plus
   committed snapshots for the pinned versions. Tests: `tools/test-provider-contract-snapshot.py`
   covers the help parser on recorded help text (hidden-flag and variadic cases), the schema
   normalizer on a recorded schema subset, and diff classification. It starts no live binary.
3. **Declared contracts, drift gate, fail-closed replies.** Add method enums and flag rows used
   by the code, and `tests/provider_contract_drift.rs`. Remove branches absent from the pinned
   schema (`turn/failed`, `turn/aborted`, `codex/event/*`, if confirmed). Reply to unknown Codex
   server requests and unknown Claude control requests with an error. Tests: the drift test;
   unknown server request gets an error reply; unknown `control_request` gets an error
   `control_response`; a deliberately altered snapshot fixture makes the drift test report the
   expected lines.
4. **Per-provider modules.** Move-only, plus the deletions in §2 and the check-architecture
   literal rule. Tests: existing suites unchanged; a check-architecture self-test for the rule;
   the Python tools consume the argv example.
5. **Command catalog.** Add the Gent table in `gent-core`, `agent-chat-commands-v1` frames,
   Claude `commands` parsing shared with the model probe, `InvokeCommand`, and `SendPrompt`
   command-shape rejection. Tests: `fixtures/ipc-contract/agent-chat-commands.json`; table
   invariants (reserved names win, `__` hidden, every built-in has a disposition); gentd
   transport tests with the fake Claude harness returning `commands`; `/nope` rejected with no
   run created; `/Users/x` accepted as a prompt; `providerNative` delivered verbatim while a goal
   is active.
6. **Compaction intent.** Codex `thread/compact/start`, and fix the compaction poll path that
   routes `Compaction` facts into `record_normalized_session` (`G:public_driver_runtime/session.rs:156`).
   Claude only after capturing its `compaction` cell. Tests: fake Codex compact records
   compaction facts and the turn settles; recorded Claude `compact_boundary` normalization.
7. **TUI cutover.** Catalog-driven `slash_command`, help from the catalog, `/fork` →
   `ForkConversation`, `gent "/x"` → `InvokeCommand`. Tests: invert
   `unknown_slash_commands_remain_provider_prompts`; real-gentd CLI tests for `/new`,
   `/resume ID`, `/fork` and unknown commands.
8. **App cutover (clouseau-app, spec first).** Delete `SlashCommandSpec`, manifest
   `slashCommands`, `AgentAdapter.slashCommands` and the `isGentd` branches. Add a pure
   `command_resolution.dart` used by the composer and Home, IPC `readCommandCatalog` /
   `invokeCommand`, mux `commandCatalog` agentdata and a `gentd-intent invokeCommand`. Tests:
   unit tests for resolution; a composer widget test (unknown command shows an error and sends
   nothing); a Home test (`/resume` opens the picker and creates no chat); a remote viewer test
   through the fake mux using the same helper.
9. **Playbook dry run.** Done for Claude 2.1.270 → 2.1.271 on 2026-09-15; §3 records the
   corrected steps and timings.
