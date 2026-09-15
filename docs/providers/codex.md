# Codex — provider knowledge map

Pinned version: `providers.codex` in `fixtures/provider-contracts/pins.json` (0.153.4 as of
2026-09-15). Snapshot: `fixtures/provider-contracts/codex/0.153.4/` (`source.json`, `cli.json`,
`launch-probe.json`, `protocol.json` — no `commands.json`/`models.json`: Codex has no
slash-command surface and `models.json` is Claude-only). Update playbook:
`docs/provider-update-playbook.md` §3. Command-catalog design: `docs/provider-commands-and-updates.md`.

Abbreviations: `D:` = `crates/gent-drivers/src/`, `G:` = `crates/gentd/src/`, `A:` = `crates/gent-adapters/src/`.

## Concern → file → symbols

| Concern | File(s) | Key symbols |
|---|---|---|
| Launch argv/env | `D:launch_spec.rs::codex_app_server_arguments()` (line 41) → `["app-server", "-c", "check_for_update_on_startup=false"]` — the ONE definition, called from `D:codex_runner.rs:132` and `G:model_catalog_codex.rs:60`<br>Turn params: `D:codex_session/types.rs` — `CodexTurnOptions` (10), `CodexTurnEffort` (28), `CodexSandboxPolicy` (39), `from_selection()`/`from_selection_with_permissions()` (48/59), `CodexSessionConfig` (217)<br>Env stripping: `D:process_node_search.rs:60-64` strips inherited `CODEX_MANAGED_BY_NPM/BUN/PNPM/VITE_PLUS`, `CODEX_MANAGED_PACKAGE_ROOT` before spawn |
| Protocol wire parsing | `D:codex_turn.rs::CodexTurnDriver` (45) — `start()`, `start_with_attachments()`, `receive()`, `submit()`, `steer()`, `interrupt()`; submodules `D:codex_turn/correlation.rs`, `D:codex_turn/facts.rs`<br>`D:public_protocol/codex_protocol_support.rs` — `tool_kind()` (11), `inert_item()` (26), `housekeeping()` (35, ~19 methods deliberately ignored, incl. `skills/changed`)<br>One `initialize` request: `D:message_encoding.rs::codex_initialize_request` (33) sends `capabilities:{experimentalApi:true, requestAttestation:false}` for both `encode_codex_handshake` (23) and `D:codex_session.rs::CodexAppServerSession::start` |
| Unknown server-request reply | `D:codex_client_request.rs::respond_to_codex_client_request()` (24), `reject_unhandled_codex_request()` (78) — already replies `-32601` JSON-RPC error for unknown server requests |
| Steer | `D:codex_session/steer.rs::CodexSteerOutcome{Consumed{message_id},Rejected{message_id}}`, `CodexSteers` (`pub(super)`), `CodexSteers::request()` |
| Interrupt/cancel (two distinct paths) | Steady-state user interrupt: `D:codex_session/interrupt.rs::request()` sends JSON-RPC `turn/interrupt {threadId, turnId}`, via `G:codex_prompt_lifecycle/interrupt.rs::interrupt()` (31) → `codex_runner.rs::interrupt_turn()` (208)<br>OS-signal: `D:codex_prompt_runner.rs:275::interrupt()` calls `signal_process(run_id, ProcessTreeSignal::Interrupt)` — reached only for launch-rollback cleanup (`G:codex_prompt_lifecycle/start.rs:72,141`), not the normal user-triggered path |
| Permission relay | `D:codex_control.rs::CodexControlRequest` (13), `CodexControlDecision` (23), `parse()` (28), `encode()` (125); `D:codex_control/answers.rs::question_answers()`, `D:codex_control/redaction.rs::dynamic_tool_name()` (both `pub(super)`, no public API here)<br>`D:codex_client_request.rs` (see above) |
| Usage/token parsing | `D:public_protocol/codex_protocol/stream.rs:80::token_usage()` builds `TokenUsage{...}` (line 97) |
| Model listing / auth probing | `G:model_catalog_codex.rs::CodexModelSource` (22, `provider()/label()/ttl()/revision()/load()`), `ModelListPages` (73, `accept()` 79), `model_list_request()` (121), `codex_model()` (133, parses `model/list` entries)<br>`G:provider_auth_process.rs::login_arguments()` (162) returns `&["login"]` for Codex, `classify()` (87) text-matches `login status` output |
| Skills / no slash commands | No dispatch exists yet: `skills/changed` and `thread/goal/{updated,cleared}` are ignored `housekeeping()` methods only (`codex_protocol_support.rs:45,74-75`). `skills/list`, `review/start`, `thread/fork`, `thread/rollback` and `UserInput{type:"skill"}` have **zero references anywhere** in `gent-drivers`/`gentd` — turn text is literal, unimplemented per playbook §4 step 5 |
| Compaction | `D:codex_session/compaction.rs::is_compaction()` (7), `start_compaction()` (12, sends `thread/compact/start`), `settled()` (45), `announced()` (83). `G:public_driver_runtime/session.rs::record_normalized_session()` (85) routes `PublicWireFact::Compaction(_)` (lines 28-32, 133, 181) to the dedicated compaction ingress — the file exists; the playbook's `:156` line reference is stale (current line is 85) |
| Session recovery/resume fallback | `D:codex_session/wire.rs::thread_request()` (22) branches on `thread/resume` (36); `D:codex_session/responses.rs::response()` (10), `confirm_resume_unavailable()` (63), `settle()` (98). Tests: `G:codex_prompt_lifecycle_resume_tests.rs`. Ledger events (`runExecutableRebound`, `runSessionRetired`, `providerSessionRecovered`) live in `crates/gent-store/src/sqlite/leases.rs` and `crates/gent-types/src/bounded_text.rs` — **cross-provider, not in gent-drivers/gentd codex files** |
| Provider upgrade rebind | `G:codex_authority_composition.rs::PrivateCodexAuthorityConfig`/`PrivateCodexAuthorityHost` (44/50), `compose_private_codex_authority()` (147), `respond_codex_permission()` (111)<br>`G:codex_authority_preflight.rs::CodexAuthorityPreflight` (16), `load()` (75), `verify()` (87)<br>`G:codex_authority_supervisor.rs`, `G:codex_standalone_authority.rs` |
| Install/provisioning, package policy | `D:npm_pack_install.rs::VerifiedNpmInstaller`, `install()` (29), `verify_integrity()` (131, SRI); `D:installer.rs::install_package()` (68) — shared with Claude. Package `@openai/codex` (`A:provider_platform.rs:3` `CODEX_PACKAGE`); native path `vendor/<triple>/bin/codex` built by `A:provider_platform.rs::executable()` (98-109) and mirrored in `G:standalone_provider_setup.rs:42`; version/integrity/digest in `pins.json`. Extra shipped binaries `codex-code-mode-host`/`codex-path/rg`/`codex-resources/zsh` have **no Rust-source reference anywhere** — covered only by install-time SRI, never re-verified per launch |
| Native binary verification | Shared, not Codex-only: `G:provider_executables.rs::ProviderExecutables` (`installed_readiness()`, `executable()`, `revision()`, `locks()`, `launch_lock()`), `G:standalone_provider_readiness.rs::StandaloneProviderReadinessAuthority`. `tools/provider-native-digest.py codex` |
| Fake executables / test harnesses | `G:codex_fake_cli_harness.rs::FakeCodexDaemon` — `start()` (45), `restart()` (71), `lose_thread()` (83), `reject_resumes()` (88), `prompt()` (119), `drive_until()` (158), `requests()` (231). Backing script `crates/gentd/testdata/fake-codex-app-server.py` (logs `CODEX_MANAGED*` stripping at line 168) |
| Contract snapshot / drift test | `tools/provider-contract-snapshot.py codex --executable EXE`.<br>`crates/gent-drivers/tests/provider_contract_drift.rs::{codex_findings(),codex_launch_findings()}`. `codex_findings()` scans method/field string literals in every `is_codex_source()` file against `protocol.json`'s method schema (a literal scanner, not a declared enum list) and probes real notifications/server-requests through `CodexTurnDriver` to classify `unsupportedCodexNotification`/`unsupportedCodexServerRequest`. `codex_launch_findings()` checks `launch_spec::codex_app_server_arguments()` against the pinned `cli.json` + `launch-probe.json` |
| Live verification tools | `tools/capture-codex-app-server-transcript.py`, `tools/capture-codex-mcp-transcript.py`, `tools/capture-codex-subagent-transcript.py`, `tools/public_driver_codex_appserver.py`<br>`tools/verify-live-driver-parsing.py codex --model gpt-5.6-luna` |

## If X changes in a new Codex version…

| Trigger | Edit | Run |
|---|---|---|
| A used `app-server` flag/subcommand renamed or removed | `D:launch_spec.rs::codex_app_server_arguments()` | `cargo test -p gent-drivers --test provider_contract_drift` |
| A sent/handled JSON-RPC method disappears from the schema (e.g. today's stale `turn/failed`/`turn/aborted`/`codex/event/*` cleanup) | Delete/replace the branch in `D:public_protocol/codex_protocol*.rs`, `D:codex_turn*.rs`, or `D:codex_session/notifications.rs`; delete tests exercising only that branch | `cargo test -p gent-drivers --test provider_contract_drift` |
| A new server notification appears | Handle it in `D:public_protocol/codex_protocol.rs` or add to `housekeeping()` in `D:codex_protocol_support.rs` with one line of reasoning in the commit | drift test (fails until classified) |
| A new server request appears | Answer it in `D:codex_control.rs` or `D:codex_client_request.rs`; confirm the `-32601` fail-closed path still covers anything left unhandled. If it gates approvals/auth: HUMAN | drift test + a live smoke (playbook §3 step 8) |
| A `model/list` field changes | `G:model_catalog_codex.rs::codex_model()` | `cargo test -p gentd model_catalog` |
| Version bump itself | `fixtures/provider-contracts/pins.json` (`providers.codex`), snapshot dir, `python3 tools/provider-native-digest.py codex <target> W/<tgz>` | Full gate: playbook §3 steps 3–8 |

## Notes for a future cleanup (report only, not applied here)

- Two interrupt paths (native `turn/interrupt` vs OS `ProcessTreeSignal::Interrupt`) are both live.
- `codex_control/answers.rs` and `codex_control/redaction.rs` expose no `pub` items — correct, but
  worth noting so a future agent doesn't look for a public API there.
