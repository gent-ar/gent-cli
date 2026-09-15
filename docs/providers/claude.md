# Claude Code — provider knowledge map

Pinned version: `providers.claude` in `fixtures/provider-contracts/pins.json` (2.1.271 as of
2026-09-15). Snapshot: `fixtures/provider-contracts/claude/2.1.271/` (`source.json`, `cli.json`,
`commands.json`, `models.json`, `protocol.json`, `launch-probe.json`, `frames.json`). Update
playbook: `docs/provider-update-playbook.md` §3. Command-catalog design: `docs/provider-commands-and-updates.md`.

Abbreviations: `D:` = `crates/gent-drivers/src/`, `G:` = `crates/gentd/src/`, `A:` = `crates/gent-adapters/src/`.

## Concern → file → symbols

| Concern | File(s) | Key symbols |
|---|---|---|
| Launch argv/env | `D:launch_spec.rs` — `LaunchIntent{Start,Resume{session_id},Recreate{session_id}}`, `arguments()`, `claude_stream_arguments()`, `append_claude_mcp_config()`<br>`D:claude_turn_options.rs` — `ClaudeTurnOptions`, `ClaudePermissionMode`, `ClaudeTurnMode`, `from_selection_with_permissions()`, `append_arguments()`, `claude_effort()`<br>`D:claude_runner.rs:118-135` assembles the full argv<br>`D:claude_runner_input.rs::follow_up_input_frame()`<br>`G:claude_summary_runner.rs::ClaudeSummaryRunner` (reuses `launch_spec::arguments` + `ClaudeTurnOptions::summary()`) |
| Resume / session id | `--resume <id>` (Resume) vs `--session-id <id>` (Recreate) in `launch_spec.rs`; `D:claude_runner_input.rs` |
| Protocol wire parsing | `D:public_protocol.rs::claude()` (line 65, dispatch by `type`/`subtype`), `claude_stream_event()`, `claude_stream_block_start()`, `claude_stream_delta()`, `claude_assistant()`, `claude_content()`, `claude_error()`<br>`D:public_protocol/claude_protocol.rs` + `control.rs`, `child.rs`, `usage.rs` |
| control_request/control_response, unknown-request reply | `D:claude_control.rs` — `ClaudePermissionRequest`, `ClaudePermissionBehavior`, `parse_permission_request()`, `encode_permission_response()`, `encode_permission_response_with_input()`, `encode_plan_review_response()`, `control_request_id()`, `encode_control_error()`<br>Fail-closed dispatch: `D:claude_runner_frames.rs::reject_control_request()` calls `encode_control_error()` for any unrecognized/duplicate/malformed control request — unknown subtypes DO get a `control_response` error today |
| Steer | `G:claude_prompt_lifecycle/steer.rs` — `steer_active()`, `consume_steer()`, `settle_turn()`, `start_steer_turn()`, `release_steers()`<br>`D:claude_runner.rs::steer()` (driver-level send, ~line 172) |
| Interrupt/cancel | `D:claude_runner.rs::signal()` (`ProcessTreeSignal`, shared `D:interrupt.rs`)<br>`G:claude_prompt_lifecycle/execution.rs::interrupt()` → `runner.signal(run_id, ProcessTreeSignal::Interrupt)`<br>`D:claude_runner_frames.rs` replays the interrupt echo frame |
| Permission relay | `D:claude_permission_relay.rs::ClaudePermissionRelay` (`pub(crate)`) — `accept()`, `response_with_input()`, `settle()`, `cancel()` (keeps provider-native suggestions private, never returned)<br>`G:claude_prompt_lifecycle/permission.rs` — `ClaudePermissionRecord`, `record_permission_request()`, `respond_permission()`, `respond_permission_with_input()`. Persistent/session grants are the `updatedPermissions` field echoed by `encode_permission_response_with_input()` |
| Usage/token parsing | `D:public_protocol/claude_protocol/usage.rs` (27 lines) — `context_usage()`, `turn_usage()`, `token_usage()` |
| Model listing / auth probing | `G:model_catalog_claude.rs::ClaudeModelSource`, `claude_model()`<br>`G:model_catalog_claude_initialize.rs::ClaudeInitialize`, `InitializeState`, `load()`, `state()`, `probe()`, `initialize_response()` — **one `initialize` probe serves both `models` and `commands`, one TTL**<br>`G:provider_auth_process.rs::login_arguments()` (`&["auth","login"]`), `classify()`, `launch()` |
| Slash-command catalog classification | `D:claude_commands.rs` — `ClaudeBuiltin{Native,Unsupported(reason,use),GentReserved}`, `CLAUDE_BUILTIN_COMMANDS` (const disposition table, one row per built-in), `claude_builtin(name)`, `claude_command_descriptors(initialize_json)` (parses `initialize`'s `commands` array; names not in the table are provider skills). Tests: `D:claude_commands_tests.rs` |
| Compaction | `D:public_protocol.rs` line ~83: `("system", "status"\|"compact_boundary")` → `claude_protocol::compaction(frame)` in `D:public_protocol/claude_protocol.rs` (maps `compact_boundary`→`Completed`, an error frame→`Failed`, else→`Started`). Facts land in `G:private_compaction_ingress.rs::record_observation()`/`record()` — **that ledger file is shared with Codex**, only the wire mapping above is Claude-specific. `/compact` dispatch text ("no arguments; this provider compacts without instructions") is in `G:runtime_facade_command_intents.rs:68` — a third file in the `/compact` story, alongside the disposition row in `D:claude_commands.rs` |
| Session recovery/resume fallback | `G:claude_prompt_lifecycle/execution.rs::resume()` (rejects empty session_id), `::start()`<br>`G:claude_prompt_lifecycle/recovery.rs::recover_unavailable_session()` (`pub(super)`)<br>Tests: `G:claude_session_recovery_tests.rs`, `G:claude_session_continuity_tests.rs` |
| Provider upgrade rebind | `G:claude_authority_composition.rs::compose_private_claude_authority()`, `PrivateClaudeAuthorityHost`<br>`G:claude_authority_preflight.rs::ClaudeAuthorityPreflight::{load,verify}()`<br>`G:claude_authority_supervisor.rs::PrivateClaudeSupervisor::{wake,interrupt_run}()`<br>`G:claude_standalone_authority.rs::StandaloneClaudeHost`<br>The `runExecutableRebound`/`runSessionRetired`/`providerSessionRecovered` events themselves are **cross-provider primitives**, not Claude-owned: `crates/gent-store/src/sqlite/leases.rs`, diagnostic constants in `crates/gent-types/src/bounded_text.rs` |
| Install/provisioning, package policy | `D:npm_pack_install.rs::VerifiedNpmInstaller::{install,verify_integrity}()` (SRI check)<br>`D:installer.rs::{NpmGlobalPrefix,pack(),install_archive(),install_package(),configure_command()}` — shared with Codex, package name is data (`pins.json`)<br>Package identity: `A:provider_platform.rs::CLAUDE_PACKAGE_PREFIX = "@anthropic-ai/claude-code-"` (4), `executable("claude")` (98) maps platform → binary path |
| Native binary verification | Shared, not Claude-only: `G:provider_executables.rs::ProviderExecutables` (`installed_readiness()`, `executable()`, `revision()`, `locks()`, `launch_lock()`), `G:standalone_provider_readiness.rs::StandaloneProviderReadinessAuthority`. `D:node_runtime_lock.rs` for the bundled Node runtime digest. `tools/provider-native-digest.py` takes `--provider claude` |
| Fake executables / test harnesses | `G:claude_fake_cli_harness.rs::FakeClaudeDaemon::{start,prompt,drive_until,forget_session,fail_resumes_with_an_api_error}()`<br>`G:claude_fake_cli_harness_reads.rs::{phase,transcript,projection,launches,sessions,bound_session,session_prompts}()` |
| Contract snapshot / drift test | `tools/provider-contract-snapshot.py claude --executable EXE` writes the snapshot dir above.<br>`crates/gent-drivers/tests/provider_contract_drift.rs::{claude_findings(),claude_command_findings()}`, combined in `pinned_provider_contracts_match_what_the_drivers_depend_on`. `claude_findings()` scans literal `--flags` in `launch_spec.rs`/`claude_turn_options.rs` against `cli.json`+`launch-probe.json`, and walks `provider_auth_process.rs`'s `ProviderAuthProvider::Claude` argument lists against `cli.json` subcommands/options. `claude_command_findings()` diffs `commands.json` against `CLAUDE_BUILTIN_COMMANDS` |
| Live verification tools | `tools/capture-claude-mcp-transcript.py`, `tools/capture-claude-persistent-permission-transcript.py`, `tools/capture-claude-subagent-transcript.py` (each has a paired `tools/test-capture-claude-*.py`)<br>`tools/verify-live-driver-parsing.py claude --model haiku`<br>`tools/public_driver_probes.py` |

## If X changes in a new Claude Code version…

| Trigger | Edit | Run |
|---|---|---|
| A used launch flag renamed/removed, or its arg shape changed | `D:launch_spec.rs` or `D:claude_turn_options.rs` (build side), plus the argv unit test in whichever file | `cargo test -p gent-drivers --test provider_contract_drift` |
| A new/removed `system`/`stream_event` frame `type`/`subtype` | `D:public_protocol.rs::claude()` dispatch and/or `D:public_protocol/claude_protocol.rs`; find real-world context with `strings -n 4 EXE \| grep -o '.\{160\}<subtype>.\{160\}' \| head -3` | `cargo test -p gent-drivers` |
| A new `control_request` subtype Claude sends (permission/plan-review shaped) | `D:claude_control.rs` (parse + encode), `D:claude_permission_relay.rs` if it carries suggestions, `D:claude_runner_frames.rs` dispatch | `cargo test -p gent-drivers --test provider_contract_drift`, then a live smoke (playbook §3 step 8) |
| A new/removed/renamed built-in slash command | Add/edit its row in `D:claude_commands.rs::CLAUDE_BUILTIN_COMMANDS` (`Native`, `Unsupported(reason, use?)`, or `GentReserved`) | `cargo test -p gent-drivers --test provider_contract_drift` (fails until every `commands.json` entry has a row), `D:claude_commands_tests.rs` |
| `models` shape in the `initialize` response changes | `G:model_catalog_claude.rs`, `G:model_catalog_claude_initialize.rs` | `cargo test -p gentd model_catalog` |
| A launch probe now warns on stderr, or a flag needs `--help` re-parsing | Fix the argv in `launch_spec.rs`/`claude_turn_options.rs` — never ignore a stderr warning | `python3 tools/provider-contract-snapshot.py --diff claude/OLD claude/NEW` |
| Version bump itself | `fixtures/provider-contracts/pins.json` (`providers.claude`), snapshot dir, `python3 tools/provider-native-digest.py claude <target> W/<tgz>` | Full gate: playbook §3 steps 3–8 |

## Notes for a future cleanup (report only, not applied here)

- `G:permission_category.rs::for_tool()` mixes Claude, Codex, and other tool-name aliases with no
  vendor separation — the playbook's planned per-provider `tools.rs` split has not happened.
- `docs/provider-commands-and-updates.md`'s evidence section states `claude_control.rs:35-37`
  "gets no reply" for unknown control requests. Current source already fails closed via
  `encode_control_error()` (called from `claude_runner_frames.rs::reject_control_request()`) —
  that evidence note is stale and should be corrected when that doc is next touched.
- Two files are literally named `node_runtime_lock.rs`: `D:node_runtime_lock.rs` (the real
  digest/lock implementation) and `G:node_runtime_lock.rs` (a thin daemon-side wrapper +
  tests). Both are provider-neutral, but an agent grepping the filename should check which
  crate it landed in.
