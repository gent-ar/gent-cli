# Provider commands and provider updates

Status: §1 is implemented (playbook steps 5, 7, 8), including `/compact`; §2's snapshots and
drift gate exist. Companion: `docs/provider-update-playbook.md` covers where
provider knowledge lives, the version-bump playbook, and the implementation plan.

Implemented deviations from §1: `/login` and `/rename` are client actions (`providerLogin`,
`rename`) because provider auth is an interactive `provider-auth-v1` flow and Gent has no user
title write; `/compact` is per provider: Claude lists it as a provider-native builtin (its
`initialize` names it and 2.1.270 compacts in SDK mode), Codex as the Gent `compact` intent sent as
`thread/compact/start` on the bound thread, and Claurst as the Gent `compact` intent run by Gentd's
own summarizer (`docs/providers/claurst.md` "Context compaction"); Claude
`system/status`/`system/compact_boundary` and Codex `contextCompaction` normalize to compaction
observations that Gentd records once per turn as a "Context compacted" notice; rejections are typed
`error` frames (`unknownCommand`, `unsupportedCommand`, `clientActionCommand`,
`commandRequiresConversation`, `commandBlockedByActiveTurn`, `commandArgumentsInvalid`,
`commandRequiresProviderSession`, `commandCatalogLoading`, `slashCommandRequiresInvoke`), not
`CommandInvoked` outcomes; the catalog carries `listing` and clients re-read while it is loading
instead of a `commandCatalogChanged` delta. Retries are idempotent because the first invocation
records its resolved intent as an `agentChatCommandResolved` event keyed by receipt. The terminal's
switch-context command is `/context-policy`, so Claude's `/context` stays native.

Two problems, one root: Gent depends on provider CLI surfaces (flags, protocol methods, slash
commands) that change every release, and today that dependency is implicit — scattered string
literals, a client-side command list per client, and tests that pin behaviour the provider no
longer has. This design makes the dependency explicit, owned by Gentd, and checked against a
captured snapshot of every pinned provider version.

## Evidence (captured 2026-09-14, scratch under `release-work/cmd/`)

- Claude Code 2.1.270, `control_request {subtype: initialize}` on the exact driver argv
  (`launch_spec.rs` flags plus `--model haiku --permission-prompt-tool stdio`), no prompt sent:
  one `control_response` in 0.9 s, empty stderr. Response keys: `commands`, `agents`,
  `output_style`, `available_output_styles`, `models`, `account`, `current_permission_mode`,
  `fast_mode_state`, `session_state`, and remote-control flags. `commands` held 78 entries
  `{name, description, argumentHint, aliases?}`: user/plugin skills plus built-ins such as
  `clear` (aliases `reset`, `new`), `compact`, `model`, `effort`, `goal`, `context`, `usage`,
  `init`, `mcp`, `config`, `rename`, `recap`, `output-style`, `fast`, `advisor`, `agents`,
  `__remote-workflow`. There is no `resume`, `fork`, `login` or `rewind` in SDK mode.
- `claude --bogus-flag --version` exits 0, and `--effort bogus` only warns and falls back.
  `--version` acceptance is therefore not evidence that a flag exists (the precedent in
  `CLAUDE.md` used it). `--max-turns`, used by `claude_turn_options.rs`, is absent from
  `--help`. Flag evidence must come from the real launch argv plus a clean stderr.
- Codex 0.153.4 `app-server generate-json-schema`: 99 client requests, 81 server
  notifications, 10 server requests, 1 client notification. There is no slash-command surface:
  `turn/start` text is literal. Adjacent typed surfaces exist: `thread/compact/start`,
  `skills/list` + `skills/changed` + `UserInput{type: skill}`, `review/start`, `thread/fork`,
  `thread/rollback`, `thread/goal/*`.
- Drift, resolved in step 3: the drivers handled `turn/failed`, `turn/aborted` and
  `codex/event/{error,stream_error}`, read `/params/msg/*` and `error.code`/`error.type`, and
  ignored `thread/realtime/transcript`. None of these appear in the 0.153.4 schema or in the native
  binary's strings, and the app driver notes that 0.144.1 already replaced `turn/aborted` with
  `turn/completed{status: interrupted}`. That handling was removed. 17 schema notifications fell
  through to `unsupportedCodexNotification`; they are now classified. Claude `system/compact_boundary` fell
  into `unsupportedClaudeFrame`; it is now captured from 2.1.270 and normalized.
- Claurst v0.1.7 (`tools/stage-claurst-runtime.py`): ACP server handles only `initialize`,
  `authenticate`, `session/new`, `session/prompt` and `session/cancel`. It emits only
  `agent_message_chunk`, `agent_thought_chunk`, `tool_call` and `tool_call_update`, and never
  `available_commands_update`. Gent drops unknown `sessionUpdate` kinds silently
  (`claurst_acp_transport_updates.rs:8-24`).
- Gent today: the TUI hardcodes commands in `terminal/state_submit.rs:53-86`, and any
  unrecognized `/xyz` is sent as a prompt (locked in by `unknown_slash_commands_remain_provider_prompts`).
  `/fork` sends `SwitchSelection`, not `ForkConversation`, and `/compact` is not handled.
  `gent "/x"` only recognizes `/fanout`, `/cross-review` and `/goal`. The app keeps adapter
  manifest lists (`catalog_spec.dart:239` `SlashCommandSpec`, seed claude/codex
  `slashCommands`), hides them once a chat is Gentd-bound (`panel_input.dart:181`,
  `panel_send_message.dart:99`), sends `/compact` as prompt text (`panel_input.dart:383`), and
  Home turns any typed `/x` into a new chat's first prompt (`home_prompt_actions.dart`).
  Remote viewers forward raw text through `gentd-intent` `prompt`. Gentd never inspects a
  leading `/`. `goal_projection.rs` prepends a goal preamble, so even a supported provider
  command stops being a command while a goal is active.

## 1. One command catalog owned by Gentd

### Descriptor

A command is a typed descriptor served by Gentd, never a client constant:

```
CommandDescriptor { name, aliases[], description, argumentHint?, origin, dispatch, availability }
origin   = gent | providerBuiltin{provider} | providerSkill{provider}
dispatch = gentIntent{intent} | clientAction{action} | providerNative | unsupported{reason, use?}
availability = { requiresConversation, blockedWhileTurnActive }
```

- `gentIntent`: Gentd executes a receipt-backed intent (create, switch, fork, compact, goal,
  login). The provider never sees the text.
- `clientAction`: a stable action id the client renders (`conversationPicker`, `help`,
  `attach`, `search`, `thinkingToggle`). A client hides action ids it does not implement.
- `providerNative`: Gentd delivers `/name args` verbatim as its own provider turn. It skips the
  goal preamble and history wrapping, because the provider executes it in the mode Gent runs.
- `unsupported`: typed rejection naming the Gent replacement, when there is one.

### Gent-owned table (pure data in `gent-core`, one file)

| Command | Dispatch | Why |
| --- | --- | --- |
| `/new` | `gentIntent createConversation` | ledger owns conversations |
| `/resume` | `clientAction conversationPicker`, `/resume ID` → open | no provider resume in SDK/app-server/ACP |
| `/clear` (`/reset`) | `gentIntent switchSelection{context: clear}` | provider `/clear` would fork the native session behind the ledger |
| `/fork` | `gentIntent forkConversation` | fixes TUI `/fork` → `SwitchSelection` |
| `/model` `/effort` `/provider` `/mode` | `gentIntent switchSelection` | selection authority, `selectionSwitchBlockedByActiveTurn` |
| `/goal` | `gentIntent goal` | Gent goals; hides Claude's built-in `/goal` |
| `/compact [instructions]` | `gentIntent compact` | Codex: `thread/compact/start`. Claude: `/compact` user frame, only after `system/compact_boundary` is captured and normalized. Claurst: Gent summary fact via the local llama-server |
| `/login` | `gentIntent providerAuth` | provider-auth-v1 |
| `/rename` | `gentIntent` title | Gent titles |
| `/config` `/mcp` `/output-style` `/fast` `/advisor` `/autocompact` `/permissions`(provider) | `unsupported` | provider-side config diverges from Gent-owned MCP/permission/selection state |
| `/help` `/attach` `/detach` `/search` `/thinking` `/activity` `/steer` | `clientAction` / existing intents | presentation or existing queue intents |

Reserved names win over provider entries with the same name or alias. Names beginning `__` are
never exposed.

### Provider sources

- Claude: the existing `model_catalog_claude.rs` `initialize` probe already returns `commands`;
  parse both `models` and `commands` from that one response (one probe, one TTL). Probe with
  `cwd` set to the conversation workspace, because project skills depend on it, and cache by
  (workspace, executable digest). Built-ins are the names in the pinned snapshot's
  `commands.json`, captured with an empty config dir (§2). Each built-in needs a row in
  `claude/commands.rs` (`providerNative` or `unsupported`), or the drift test fails. Names not
  in the snapshot are skills and become `providerSkill` + `providerNative`.
- Codex: nothing is text-native. `skills/list` entries become `providerSkill`, dispatched as
  `UserInput{type: skill, name, path}` (a later step). Refresh on `skills/changed`.
- Claurst: none today. Parsing `available_commands_update` waits until the pinned Claurst emits
  it; the drift test raises it when a new snapshot shows it.

### Wire contract

Capability `agent-chat-commands-v1`, following `model-catalog-v1`'s frame conventions
(`{type, body}`, camelCase, `deny_unknown_fields`, bounded `validate()`):

```
ReadCommandCatalog { requestId, conversationId?, workspacePath?, refresh }
CommandCatalog     { requestId, scope, revision, providerVersion?, commands: [CommandDescriptor] }
InvokeCommand      { requestId, receiptId, conversationId, name, arguments }
CommandInvoked     { requestId, receipt, conversationId, outcome }
```

`outcome` is `delivered{runId}`, `intentApplied`, or a typed error: `unknownCommand`,
`unsupportedCommand{use?}`, `commandRequiresConversation`, `selectionSwitchBlockedByActiveTurn`.
The catalog is a read, not ledger state, like the model catalog. Its revision changes when the
provider executable digest, the workspace skill set, or the Gent table changes. The projection
follow carries `commandCatalogChanged{revision}` so clients re-read. Nothing is cached as truth.

### Enforcement rule

Gentd rejects `SendPrompt`/`QueuePrompt` whose first token is command-shaped
(`^/[A-Za-z0-9][A-Za-z0-9:_-]*(\s|$)`; `/Users/x` is not command-shaped) with
`slashCommandRequiresInvoke{name, known}`. A client never sends an unrecognized slash command as
prompt text: it resolves the token against the catalog, invokes or shows "Unknown command".
Gentd enforces this too, so the TUI, the app, remote viewers and `gent "/x"` cannot diverge.

### Consumers

- TUI: `slash_command()` becomes catalog lookup plus a `clientAction` match. Delete the
  hardcoded arms that duplicate intents. Help renders from the catalog. Invert the
  `unknown_slash_commands_remain_provider_prompts` test. `gent "/x"` goes through `InvokeCommand`.
- App (local and remote, one path): delete `SlashCommandSpec`, manifest `slashCommands`,
  `AgentAdapter.slashCommands`, and the `isGentd` branches in `panel_input.dart`,
  `panel_send_message.dart` and `home_slash_commands.dart`. One pure `command_resolution.dart`
  resolves text against the catalog for both composer and Home. A local owner reads the catalog
  over IPC; a remote viewer reads the same frames over mux (`gentd-intent` gains `invokeCommand`,
  `agentdata` gains `commandCatalog`). The Remote Parity Rule holds because resolution is one
  shared helper and only transport differs.

## 2. Provider contract snapshots and the drift gate

### Snapshot tool: `tools/provider-contract-snapshot.py`

`provider-contract-snapshot.py {claude|codex|claurst} --executable PATH [--out fixtures/...]`
writes `fixtures/provider-contracts/<provider>/<version>/`. Fixtures are exempt from the
300-line rule. Files are normalized (sorted keys, no descriptions, no timestamps inside) so a
diff shows only surface changes:

| File | Claude | Codex | Claurst |
| --- | --- | --- | --- |
| `source.json` | version, sha256, platform, captured-at, tool revision | same | same, plus ACP schema crate version |
| `cli.json` | `--help` parsed to `{subcommands, options{flag, arg: none/required/optional/variadic}}` | `--help` + `app-server --help` | `--help` |
| `protocol.json` | initialize response key set; message `type/subtype` set from the transcript corpus | method sets per direction; per-method params/result property trees | initialize fields; `sessionUpdate` kinds from Claurst's ACP schema dependency |
| `commands.json` | built-in `{name, aliases, argumentHint}` with an empty `CLAUDE_CONFIG_DIR` | — | advertised commands (none in v0.1.7) |
| `models.json` | `models[].{value, supportedEffortLevels}` | `model/list` shape only | — |
| `launch-probe.json` | stderr and first-frame result for each driver argv variant | `initialize` result for the app-server argv | none; Claurst is never run |

Probes send only `initialize` (never a prompt), use a scratch cwd, kill within 25 s, and run one
provider at a time. Claude probes run with an empty `CLAUDE_CONFIG_DIR`, so `commands.json`
holds built-ins and bundled skills only. The Claurst snapshot is static: its ACP server and prompt
sources in a checkout, plus the `agent-client-protocol-schema` crate it locks. `source.json` marks
`pinned_release_binary_verified: false` until the checkout is proven to be the pinned tag. Raw
schema output stays in scratch; only the normalized subset is committed.

### Driver-declared contract

Each provider module exposes its dependencies as data that the code itself uses, not a parallel
list. Examples: a `CodexNotification` enum with `const ALL` and `fn method()`, parsed by
iterating `ALL`, and Claude `LaunchFlag { flag, arg, hidden }` rows that build the argv.

- `LAUNCH_FLAGS` / `SUBCOMMANDS` (Claude argv, Codex `app-server`, auth argv `auth status --json`, `login status`)
- `SENT_REQUESTS`, `HANDLED_NOTIFICATIONS`, `HANDLED_SERVER_REQUESTS`, `IGNORED_NOTIFICATIONS`
- `READ_FIELDS` per method (JSON pointers the parser reads), `BUILTIN_COMMAND_DISPOSITION`

### Drift test: `crates/gent-drivers/tests/provider_contract_drift.rs`

For each provider it reads `fixtures/provider-contracts/<provider>/pinned.json` (the version the
signed compatibility entry authorizes) and fails with one line per finding:

1. A declared flag is missing from `cli.json`, or its arg shape changed. A `hidden` flag must
   instead appear accepted in `launch-probe.json`.
2. A sent/handled method is absent from the schema. This finds today's `turn/failed`,
   `turn/aborted` and `codex/event/*`.
3. A schema notification or server request is in neither `HANDLED` nor `IGNORED`. An unhandled
   server request is refused with a generic JSON-RPC error, so this is always a failure.
4. A `READ_FIELDS` pointer no longer exists in that method's property tree, or changed type.
5. A Claude built-in command has no disposition row, or a reserved Gent name lost its mapping.
6. A `launch-probe.json` stderr is non-empty (unknown flag or value warnings).

Message format: `codex 0.160.0 · notification thread/foo/updated is new → add to
HANDLED_NOTIFICATIONS (codex/protocol.rs) or IGNORED_NOTIFICATIONS (codex/contract.rs)`.
A second mode, `provider-contract-snapshot.py --diff OLD NEW`, prints the same classification
between two captured versions before any Rust changes.

### Tie-in to pinning and signing

Today the Claude/Codex pins are not in the repository at all. They exist only inside
`ordinary-authority.json`, which `release.yml:166-179` decodes from the secret
`GENT_ORDINARY_AUTHORITY_RELEASE_BASE64`. That document contains the compatibility entries
`(provider, version, digest_sha256)` (`gent-adapters/src/compatibility.rs`), the npm
`package_policy` (exact semver plus SRI), and per-provider scenario evidence. The version it
compares is raw `--version` stdout (`"2.1.233 (Claude Code)"`, `"codex-cli 0.144.1"`). The
transcript corpus was recorded at Claude 2.1.233 and Codex 0.144.1, while this machine runs
2.1.270 and 0.153.4. Claurst is pinned in `stage-claurst-runtime.py` (v0.1.7 plus per-target
sha256). The shipped standalone path does not recheck the manifest at turn start: its authorizer
checks only `standalone-local-v1` and a digest
(`public_driver_runtime_composition.rs:36-43`).

Decisions:

- Commit the unsigned pin as `fixtures/provider-contracts/pins.json`: per provider, the version,
  the `--version` string, npm package plus integrity per platform, and the Claurst tag plus
  sha256. Generate the compatibility, package-policy and authority payloads from it. Only the
  Ed25519 key remains secret. `stage-claurst-runtime.py` reads its pin from the same file.
- `sign-ordinary-authority-release.py` refuses a payload whose versions differ from `pins.json`,
  or whose pinned version has no `fixtures/provider-contracts/<provider>/<version>/` snapshot.
  It also refuses when the snapshot `source.json` executable sha256 differs from the compatibility
  entry digest.
- The drift test reads `pins.json`, never "whatever is installed", so CI is deterministic.
  A snapshot tool run against the installed CLI is the only thing that touches a live binary.
- `manifest.yml` cells carry the provider version they were captured with. The drift test lists
  `recorded` cells older than the pin, and the playbook recaptures the scenarios whose protocol
  surfaces changed.
- Unknown provider requests fail closed, so a provider never waits on Gent. `codex_turn.rs`
  answers every Codex server request it does not handle, or refuses, with a JSON-RPC error from
  `reject_unhandled_codex_request` (`codex_client_request.rs`) and records a transport
  diagnostic (`fails_closed_for_server_to_client_requests_without_wedging_the_turn`). An
  unrecognized or malformed Claude `control_request` gets a `control_response` with
  `subtype: error` from `reject_control_request` (`claude_runner_frames.rs`), except a duplicate
  permission request, whose original is still pending
  (`unrecognized_or_malformed_control_requests_are_answered_with_an_error_so_claude_never_waits`).
