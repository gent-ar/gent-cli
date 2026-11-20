# Claude Code — provider knowledge map

Pinned version: `providers.claude` in `fixtures/provider-contracts/pins.json` (2.1.271 as of
2026-09-15). Snapshot: `fixtures/provider-contracts/claude/2.1.271/` (`source.json`, `cli.json`,
`commands.json`, `models.json`, `protocol.json`, `launch-probe.json`, `frames.json`). Update
playbook: `docs/provider-update-playbook.md` §3. Command-catalog design: `docs/provider-commands-and-updates.md`.

Abbreviations: `D:` = `crates/gent-drivers/src/`, `G:` = `crates/gentd/src/`, `A:` = `crates/gent-adapters/src/`.

## Concern → file → symbols

| Concern | File(s) | Key symbols |
|---|---|---|
| Launch argv/env | `D:launch_spec.rs` — `LaunchIntent{Start,Resume{session_id},Recreate{session_id}}`, `arguments()`, `claude_stream_arguments()`, `append_claude_mcp_config()`<br>`D:claude_turn_options.rs` — `ClaudeTurnOptions`, `ClaudePermissionMode`, `ClaudeTurnMode`, `from_selection_with_permissions()`, `append_arguments()`, `claude_effort()`<br>`D:claude_runner.rs:118-135` assembles the full argv<br>`D:claude_runner_input.rs::follow_up_input_frame()`<br>`G:claude_summary_runner.rs::ClaudeSummaryRunner` (reuses `launch_spec::arguments` + `ClaudeTurnOptions::summary()`; titles, recaps and side questions. The stream replays the prompt and every partial delta, so it keeps only the final `result` frame and bounds nothing but frame size; each caller bounds its own answer) |
| Resume / session id | `--resume <id>` (Resume) vs `--session-id <id>` (Recreate) in `launch_spec.rs`; `D:claude_runner_input.rs` |
| Protocol wire parsing | `D:public_protocol.rs::claude()` (line 65, dispatch by `type`/`subtype`), `claude_stream_event()`, `claude_stream_block_start()`, `claude_stream_delta()`, `claude_assistant()`, `claude_content()`, `claude_error()`<br>`D:public_protocol/claude_protocol.rs` + `control.rs`, `child.rs`, `usage.rs` |
| control_request/control_response, unknown-request reply | `D:claude_control.rs` — `ClaudePermissionRequest`, `ClaudePermissionBehavior`, `parse_permission_request()`, `encode_permission_response()`, `encode_permission_response_with_input()`, `encode_plan_review_response()`, `control_request_id()`, `encode_control_error()`<br>Fail-closed dispatch: `D:claude_runner_frames.rs::reject_control_request()` calls `encode_control_error()` for any unrecognized/duplicate/malformed control request — unknown subtypes DO get a `control_response` error today |
| Queued prompt into the turn | `G:claude_prompt_lifecycle/steer.rs` — `steer_active()`, `message_started()`, `message_cancelled()`, `start_steer_turn()`, `release_steers()`. Every queued prompt is written into the open turn as its own user-message uuid; `command_lifecycle` `started` delivers it into the open turn (or opens its turn when none is open), `cancelled` returns it to the queue, withdraws it after a Stop, or cancels it after a Remove. Remove writes `control_request {subtype:"cancel_async_message", message_uuid}` (`D:claude_runner_input.rs::withdraw_frame`, once per write); observed on 2.1.271, a queued uuid answers `{cancelled:true}` plus `command_lifecycle cancelled` and the running turn continues; a uuid already dequeued answers `{cancelled:false}` Every write of a prompt within one process carries a fresh uuid (`D:claude_runner_messages.rs::identity()`): observed on 2.1.326, a cancelled uuid sent again is only replayed and never run<br>`D:claude_runner.rs::steer()`, `D:claude_runner_messages.rs` (uuid → message id) |
| Interrupt/cancel | Protocol interrupt: `D:claude_runner.rs::interrupt()` writes `control_request {subtype:"interrupt", cancel_queued:true}` (`D:claude_runner_input.rs::interrupt_frame`); the turn ends on Claude's own `result` and `command_lifecycle cancelled`. Observed on 2.1.326 (`interrupt_cancel_queued_v1` on `system/init`): without `cancel_queued` the response lists written, unconsumed messages under `still_queued` and Claude runs them as its next turn by itself; with it they end `command_lifecycle cancelled` and are listed under `cancelled`, so nothing runs after a Stop or a Send now<br>`G:claude_prompt_lifecycle.rs::interrupt()/interrupt_turn()` → `runner.interrupt_claude_turn()`. With no open turn the interrupt still reaches a live session; on 2.1.326 it kills background agents (`task_updated` `killed`) and the session reports `idle` (`G:claude_idle_reap_tests.rs`). Observed on 2.1.271 (round 6): the interrupt does not stop a `run_in_background` shell (`task_type:"local_bash"`); it ran to completion and Claude opened a continuation turn. `control_request {subtype:"stop_task", task_id}` kills it (`task_updated` `killed`, `task_notification` `stopped`, no continuation), so every interrupt also writes one `stop_task` per background shell (`D:claude_runner_input.rs::stop_task_frame`). A backgrounded `task_started` turns its tool into phase `background` until its `task_updated`/`task_notification` end (`D:claude_runner_frames.rs`, `docs/background-commands.md`) OS signals (`D:interrupt.rs`) are only for shutdown, where every run is marked releasing first |
| Permission relay | `D:claude_permission_relay.rs::ClaudePermissionRelay` (`pub(crate)`) — `accept()`, `response_with_input()`, `settle()`, `cancel()` (keeps provider-native suggestions private, never returned)<br>`G:claude_prompt_lifecycle/permission.rs` — `ClaudePermissionRecord`, `record_permission_request()`, `respond_permission()`, `respond_permission_with_input()`. Persistent/session grants are the `updatedPermissions` field echoed by `encode_permission_response_with_input()` |
| Usage/token parsing | `D:public_protocol/claude_protocol/usage.rs` (27 lines) — `context_usage()`, `turn_usage()`, `token_usage()` |
| Model listing / auth probing | `G:model_catalog_claude.rs::ClaudeModelSource`, `claude_model()`<br>`G:model_catalog_claude_initialize.rs::ClaudeInitialize`, `InitializeState`, `load()`, `state()`, `probe()`, `initialize_response()` — **one `initialize` probe serves both `models` and `commands`, one TTL**<br>`G:provider_auth_process.rs::login_arguments()` (`&["auth","login"]`), `classify()`, `launch()` |
| Slash-command catalog classification | While `initialize` is still loading, `claude_builtin_descriptors()` lists the table's built-ins so `/compact` resolves; skills wait for the listing. `D:claude_commands.rs` — `ClaudeBuiltin{Native,Unsupported(reason,use),GentReserved}`, `CLAUDE_BUILTIN_COMMANDS` (const disposition table, one row per built-in), `claude_builtin(name)`, `claude_command_descriptors(initialize_json)` (parses `initialize`'s `commands` array; names not in the table are provider skills). Tests: `D:claude_commands_tests.rs` |
| Compaction | `D:public_protocol.rs` line ~83: `("system", "status"\|"compact_boundary")` → `claude_protocol::compaction(frame)` in `D:public_protocol/claude_protocol.rs` (maps `compact_boundary`→`Completed`, an error frame→`Failed`, else→`Started`). Facts land in `G:private_compaction_ingress.rs::record_observation()`/`record()` — **that ledger file is shared with Codex**, only the wire mapping above is Claude-specific. Observed on 2.1.271: a manual `/compact` emits `compact_boundary.compact_metadata{pre_tokens, post_tokens}`, where `pre_tokens` matches the full context but `post_tokens` excludes the system prompt and tools, so Gentd forwards no `contextUsage` for it. `/compact` dispatch text ("no arguments; this provider compacts without instructions") is in `G:runtime_facade_command_intents.rs:68` — a third file in the `/compact` story, alongside the disposition row in `D:claude_commands.rs` |
| No retry | Gent never resends a prompt. A failed turn is reported once as a typed `ProviderFailure`; the user decides whether to send again |
| Expired sign-in | Claude reports it only on the assistant frame (`error:"authentication_failed"`, a `<synthetic>` model message); the following `result` frame carries `is_error:true` with no typed reason. `D:claude_runner_frames.rs::normalize` remembers the assistant flag and rewrites that turn's `ProviderFailure` to `Authentication` via `D:public_protocol/claude_protocol.rs::sign_in_required`; an API error frame with `error.type:"authentication_error"` maps the same way. `G:public_driver_runtime/session_mapping.rs::transcript_event_id` prefixes the notice id with `provider-auth-required:`, so the transcript notice carries `failureCode:"providerAuthRequired"`. Tests: `G:claude_auth_notice_tests.rs` |
| Session recovery/resume fallback | `G:claude_prompt_lifecycle/execution.rs::resume()` (rejects empty session_id), `::start()`<br>`G:claude_prompt_lifecycle/recovery.rs::recover_unavailable_session()` (`pub(super)`)<br>Tests: `G:claude_session_recovery_tests.rs`, `G:claude_session_continuity_tests.rs` |
| Provider upgrade rebind | `G:claude_authority_composition.rs::compose_private_claude_authority()`, `PrivateClaudeAuthorityHost`<br>`G:claude_authority_preflight.rs::ClaudeAuthorityPreflight::{load,verify}()`<br>`G:claude_authority_supervisor.rs::PrivateClaudeSupervisor::{wake,interrupt_run}()`<br>`G:claude_standalone_authority.rs::StandaloneClaudeHost`<br>The `runExecutableRebound`/`runSessionRetired`/`providerSessionRecovered` events themselves are **cross-provider primitives**, not Claude-owned: `crates/gent-store/src/sqlite/leases.rs`, diagnostic constants in `crates/gent-types/src/bounded_text.rs` |
| Install/provisioning, package policy | `D:npm_pack_install.rs::VerifiedNpmInstaller::{install,verify_integrity}()` (SRI check)<br>`D:installer.rs::{NpmGlobalPrefix,pack(),install_archive(),install_package(),configure_command()}` — shared with Codex, package name is data (`pins.json`)<br>Package identity: `A:provider_platform.rs::CLAUDE_PACKAGE_PREFIX = "@anthropic-ai/claude-code-"` (4), `executable("claude")` (98) maps platform → binary path |
| Native binary verification | Shared, not Claude-only: `G:provider_executables.rs::ProviderExecutables` (`installed_readiness()`, `executable()`, `revision()`, `locks()`, `launch_lock()`), `G:standalone_provider_readiness.rs::StandaloneProviderReadinessAuthority`. `D:node_runtime_lock.rs` for the bundled Node runtime digest. `tools/provider-native-digest.py` takes `--provider claude` |
| Fake executables / test harnesses | `G:claude_fake_cli_harness.rs::FakeClaudeDaemon::{start,prompt,drive_until,forget_session,fail_resumes_with_an_api_error}()`<br>`G:claude_fake_cli_harness_reads.rs::{phase,transcript,projection,launches,sessions,bound_session,session_prompts}()` |
| Contract snapshot / drift test | `tools/provider-contract-snapshot.py claude --executable EXE` writes the snapshot dir above.<br>`crates/gent-drivers/tests/provider_contract_drift.rs::{claude_findings(),claude_command_findings()}`, combined in `pinned_provider_contracts_match_what_the_drivers_depend_on`. `claude_findings()` scans literal `--flags` in `launch_spec.rs`/`claude_turn_options.rs` against `cli.json`+`launch-probe.json`, and walks `provider_auth_process.rs`'s `ProviderAuthProvider::Claude` argument lists against `cli.json` subcommands/options. `claude_command_findings()` diffs `commands.json` against `CLAUDE_BUILTIN_COMMANDS` |
| Live verification tools | `tools/capture-claude-mcp-transcript.py`, `tools/capture-claude-persistent-permission-transcript.py`, `tools/capture-claude-subagent-transcript.py` (each has a paired `tools/test-capture-claude-*.py`)<br>`tools/verify-live-driver-parsing.py claude --model haiku`<br>`tools/public_driver_probes.py` |

## Run lifecycle

- Prompt identity: every prompt and steer is written with its own user-message uuid.
  `command_lifecycle` (`started`/`cancelled`) and `result.user_message_uuids` decide which Gent
  prompt a turn consumed (`D:claude_runner_messages.rs`).
- Session state: launches set `CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS=1`, so Claude reports
  `system/session_state_changed` (`running`, `requires_action`, `idle`). `idle` is the only
  signal that the session has no background work left.
- One liveness value: `G:provider_run_state.rs::RunTurns` holds whether the prompt turn or a
  provider turn is open, whether the session is running and whether a release is in progress;
  `RunLiveness` adds pending steers. The reaper, `needs_poll`, dispatch eligibility and pool
  eviction all read it, with the same rules as Codex (`G:provider_liveness_parity_tests.rs`).
- Provider-initiated turns: a `result` ends a turn, not the run. A `system/init` with no
  written message started (`ContinuationStarted`, e.g. a `task-notification` continuation)
  opens a provider turn through `NormalizedSessionBatchLedger::start_provider_turn`; its
  `result` (origin `task-notification`) or the session reporting `idle` settles it.
- A permission request that arrives while no turn is open (a background sub-agent) is asked
  like any other, without marking the root turn waiting (`RunTurns::place_request`). Nothing
  is auto-denied after a `result`.
- Typed sub-agent events: `task_started` (`is_backgrounded`), `task_progress`,
  `task_updated`/`task_notification` become `ChildStarted`/`ChildPhase`/`ChildTerminal`
  (`D:claude_runner_frames.rs::task_facts`). Children are not run liveness.
- Rate limits: the latest `rate_limit_event.rate_limit_info.resetsAt` rides on the
  `RateLimited` `ProviderFailure` as `resetsAt` (Unix seconds); absent when Claude sent none.
- Plan mode is native `--permission-mode plan` plus Gent's plan instruction and
  `permissions.ask: ["Bash","PowerShell"]`. Observed on 2.1.271: native plan auto-allows read-only
  shell commands (`ls`, `git status`) and every write to its own `~/.claude/plans/*.md` file;
  the ask rule sends every shell command to the callback, while `"*"` would also send the plan
  file and Gent would refuse it. A project-file `Write` reaches the callback and Gent refuses it
  (`PlanModeEdit`); `Agent` spawns without one. Without the instruction the model refuses
  commands and sub-agents on its own reading of Claude's plan reminder.

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
