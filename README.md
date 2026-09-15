# Gent CLI

`gent-cli` is the open-source Rust host runtime for Gent. `gent` is a standalone terminal client
that starts and connects to its local `gentd` authority; `gentd` owns durable conversations,
provider processes, local-model state, and the negotiated local IPC used by the native app.

## Current milestone

The standalone product profile is the default path for `gent`. The implemented runtime includes:

- `gentd`: a supervised local daemon using a Unix socket on macOS/Linux and a
  named pipe on Windows.
- `gent`: a local client that starts `gentd --standalone-authority` on demand.
- Claude and Codex provider hosts that are installed lazily when selected, plus
  a Claurst local-runtime path using curated ungated models and llama.cpp.
- Version negotiation, capability intersection, host-epoch fences, idempotent
  command receipts, bounded cursor-ordered durable reads, and fact replay. No
  snapshot/recovery-cache/mirrored-state/replacement layer exists; current state is fact-derived.
- SQLite-backed host state, a read-only `gent doctor` dependency report, and negotiated
  `gent conversation list`, `gent conversation status`, `gent conversation timeline`, and
  `gent conversation content` reads. `gent conversation activity` is a protocol-only future
  authority reader; observer-mode `gentd` deliberately declines its capability.
- Explicit `gent deps` plans and receipt-fenced consent requests. The shipped observer rejects
  every external install; an approved host uses app-supplied Node but privately `npm -g` installs
  signed exact packages under `.gentd`, invoking npm's locked CLI through that exact Node—not
  a system Node from `PATH`—and marks an interrupted effect `unprovable` rather than replaying it.
- Durable conversation → run → turn identity and restart-safe provider-switch lineage, exposed
  only through the capability-gated read protocol in `gentd`; timeline reads exclude all message
  content and provider-native session identifiers.
- A receipt-backed user-prompt ledger that atomically assigns an active turn and retains text outside
  receipt/event payloads. Standalone authority accepts create, send, queue, resume, and switch requests,
  then drives the selected provider through its retained lifecycle owner.
- Durable workspace → repository → worktree identities, stored independently from worktree
  leases and any future Git execution.
- Durable worktree-scoped Git-operation lifecycle records with optimistic, terminal-safe transitions;
  recording an operation never starts a Git process.
- Durable credential-free tool-source declarations for MCP, built-in, and host integrations;
  declaring one neither connects to nor starts a tool source.
- Durable ordered run checkpoints with monotonic event cursors and SHA-256 state references;
  checkpoint records never contain opaque provider state or resume a live process.
- Versioned, append-only permission policies with Default, Plan, Auto-Accept Edits, Autonomous,
  and persistent Bypass modes plus canonical exact-tool/category approvals. One confirmation selects
  Bypass; later normal Gent/App connections reuse it. Broad execution requires [OS containment](docs/sandboxing.md).
- Additive, provider-neutral lifecycle signals for thinking, compacting, permission/question
  waits, subagent work, command work, and attention; these are durable status foundations, not
  a claim that a live provider is attached to the daemon. Root generation activity is explicit,
  so waiting on detached work is never inferred from a root turn phase alone.

### Intended product boundary

Gent owns the agent-chat runtime: durable conversations, sessions, prompts, Claude and Codex provider
orchestration, the Claurst local-runtime bridge, MCP configuration, and authorized Git work. A Flutter integration
invokes `gent`/`gentd` through this local protocol; it never spawns a provider binary directly. A long-lived negotiated
`gentd` connection is the future app boundary; `gent` may launch or diagnose that host, but it is not a per-prompt
process substitute for subscriptions, receipt retries, or cursor resumption. `gentd` is the only component allowed to compose those capabilities.

The public protocol uses strictly typed, capability-gated agent-chat reads and intents. `gent` starts the standalone
authority by default. Running `gentd` without `--standalone-authority` remains an observer-only diagnostic profile;
it is not the normal CLI or native-app path.

Every new chat conversation supplies a local workspace path (`gent` defaults it to the terminal current directory; `gent chat create --workspace PATH` overrides it). `gentd` canonicalizes that path once, then atomically binds the derived Gent workspace record to the conversation/root run. The accepted prompt path uses only that durable binding; it never treats the daemon cwd as project state. This is a safety foundation, not an advertised provider launcher.

### Milestone scope contract

- `gent-cli` is the only CLI surface; it is a thin protocol client and must remain a composition boundary, not a hidden authority plane.
- `gentd` composes agent-chat domains. Standalone authority owns provider process lifecycle and MCP injection;
  the observer profile remains intentionally unable to launch provider work.
- Modules do not import each other as peers; only `gentd` and `gent` are allowed to
  compose stable interfaces across boundaries.
- Claude/Codex transport is declarative and typed; standalone authority launches only Gent-owned selected executables.
- Claurst is a local Gent provider. It uses no hosted endpoint or app credential.
- Device pairing and application-specific UI automations are Flutter-app responsibilities; a later agent-chat `gent-automations` domain stays separate and port-bound.
- [`drivers_transcript/`](drivers_transcript/README.md) is a committed, development-only corpus; normal Gent runs never record there.
  Capture test or real sessions only when a developer explicitly opts in, then sanitize and review them before committing them; corpus records never authorize providers or count as public evidence.
  Before commit, validate and offline-replay the reviewed corpus with `python3 tools/validate-driver-transcript-corpus.py` and `python3 tools/replay-driver-transcript-corpus.py`; neither command contacts a provider or enables capture.
- No source file in this milestone should exceed 300 lines; `python tools/check-architecture.py` enforces it.
`fixtures/ipc-contract/manifest.json` is the language-neutral compatibility fixture for that
local protocol. It records canonical JSON and exact four-byte big-endian length-prefixed frames
for handshake, errors, event resume, and reserved agent-chat values. A future Flutter client can
validate its codec without embedding Rust or treating reserved capability as available:

```sh
cargo run -p gent-testkit --bin validate-ipc-fixtures -- fixtures/ipc-contract/manifest.json
```

## Try it

Install a signed release (or explicitly rerun with `--force` to update it).
This is a paired manual installer update by default; no daemon updates itself in-process.
Choose a published tag, download the bootstrap asset and its Sigstore bundle,
and verify the bootstrap before executing it:

```sh
version=vX.Y.Z
curl -fLO "https://github.com/gent-ar/gent-cli/releases/download/$version/gent-install.sh"
curl -fLO "https://github.com/gent-ar/gent-cli/releases/download/$version/gent-install.sh.sigstore.json"
cosign verify-blob gent-install.sh --bundle gent-install.sh.sigstore.json \
  --certificate-identity-regexp "^https://github.com/gent-ar/gent-cli/.github/workflows/release.yml@refs/tags/$version$" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
sh gent-install.sh --version "$version"
```

The installer requires `curl`, `python3`, `tar`, and `cosign`; it verifies the
GitHub OIDC signature and the archive manifest before it installs either binary.
Set `GENT_VERSION=vX.Y.Z` to pin a release, or `GENT_INSTALL_DIR` to change the
default `~/.local` install root.

On Windows x86_64, use the signed PowerShell bootstrap (the default runtime
root is `%LOCALAPPDATA%\Gent`):

```powershell
$version = "vX.Y.Z"
$base = "https://github.com/gent-ar/gent-cli/releases/download/$version"
Invoke-WebRequest "$base/gent-install.ps1" -OutFile gent-install.ps1
Invoke-WebRequest "$base/gent-install.ps1.sigstore.json" -OutFile gent-install.ps1.sigstore.json
cosign verify-blob gent-install.ps1 --bundle gent-install.ps1.sigstore.json --certificate-identity-regexp "^https://github.com/gent-ar/gent-cli/.github/workflows/release.yml@refs/tags/$version$" --certificate-oidc-issuer https://token.actions.githubusercontent.com
.\gent-install.ps1 -Version $version
```

The Windows installer requires `cosign` and uses a ZIP archive. It atomically
replaces a validated `current.json` pointer only after both `gent.exe` and
`gentd.exe` have been staged. Add its `bin` directory to `PATH`. Do not use an
unverified `Invoke-Expression`/pipe bootstrap in high-assurance deployments.

```sh
cargo build -p gent-cli -p gentd
target/debug/gent doctor
target/debug/gent update check
target/debug/gent status
target/debug/gent --conversations
target/debug/gent conversation list
target/debug/gent submit --kind ping --payload '{"message":"hello"}'
target/debug/gent events
target/debug/gent events --follow
```

On Windows, use `target\\debug\\gent.exe` for these commands. The client starts the
matching `gentd` beside the built executable; set `GENTD_BIN` when using a different daemon.

The default data directory is `~/.gentd`. Upgrading from a build that used `~/.gent-cli` migrates
that directory into place automatically the first time `gentd` starts without an explicit
`--data-dir`/`GENT_DATA_DIR`.
Set `GENT_DATA_DIR` to an empty temporary directory when experimenting or testing.
Set `GENTD_BIN` to a specific daemon binary when `gent` should not resolve a sibling executable.
Pass `--no-autostart` to require an already-running daemon, which is useful for
supervised deployments and deterministic smoke tests.

`gent update check` is read-only and requires the explicit supervised daemon
profile; the ordinary observer daemon does not advertise it. For a verified
update, the external handoff verifies its tag-bound bootstrap, archive, manifest,
and supplied digest before staging the immutable pair. It refuses a live host,
health-checks the successor, and rolls back a failed pointer switch:

```sh
digest='target archive digest from the signed manifest'
gent --data-dir "$GENT_DATA_DIR" update apply \
  --version vX.Y.Z \
  --expected-sha256 "$digest" \
  --consent
```

Pass `--install-dir DIR` when needed. It never selects `latest`, silently
substitutes an archive, or replaces a live process in place. On an installed
pair, automatic checks are enabled by default and external:

```sh
gent update auto enable --interval-seconds 21600
gent update auto status
gent update auto disable
```

`gent update auto run` performs a one-shot check. Its LaunchAgent (macOS),
systemd-user timer (Linux), or per-user Scheduled Task (Windows) treats GitHub
`latest` only as an untrusted stable-tag hint, then repeats the signed idle-lock,
health-check, and rollback path. It serializes/backoffs; never starts provider
work or replaces `gentd` in process.
## Run standalone chat

`gent` with no arguments opens the terminal client; `gent --conversations` opens it for browsing only (no
prompts). `gent "PROMPT"` streams one reply: answer text goes to stdout, while tool activity, waiting
permissions (with ready-to-run approve/deny commands), model downloads, and failures go to stderr. Add
`--json` to `gent "PROMPT"`, `chat send`, or `chat resume` for the raw daemon frames. The local provider is
named `gent`; a provider alone picks that provider's default model.

```sh
gent --provider claude "Inspect this workspace"
gent chat queue --conversation-id CONVERSATION_ID --text 'Also check the tests'
gent chat steer --conversation-id CONVERSATION_ID
gent chat cancel-queued --conversation-id CONVERSATION_ID --all
gent chat switch --conversation-id CONVERSATION_ID --effort low
gent chat interrupt --conversation-id CONVERSATION_ID
```

In the terminal client, Enter during a turn queues, Ctrl+R (`/steer`) delivers queued prompts now, Ctrl+K
(`/cancel-queued`) removes the newest, Ctrl+C interrupts, Esc clears, End follows, Ctrl+Q quits. Exit codes: 0
ok, 1 rejected, 2 usage, 3 gentd unavailable, 4 not found, 5 consent required, 6 turn failed, 7 turn or
model download interrupted. `gent chat switch` creates a Gent-owned child run that keeps history unless
`--context clear`; a new model keeps the current effort only if it accepts it, and Gentd rejects an unsupported
`--effort` with the model's choices. Claude and Codex install only when first selected; the waiting client prints
the reviewed package and the exact `gent provider provision ... --consent` command. For a supervised daemon or shared MCP config, run
`gentd --data-dir "$GENT_DATA_DIR" --standalone-authority --mcp-config /absolute/path/mcp.json` and pass
`--no-autostart` to clients.
The MCP file must be a bounded JSON object with a `mcpServers` map of stdio server declarations.
Gentd projects the same configuration to Claude, Codex, and Claurst at provider launch.

For local Gent models, select a curated model returned by `gent models list`, then create or switch to
`--provider gent --model MODEL_ID`. The first prompt automatically downloads a missing model from its
curated Hugging Face source and reports correlated progress; `gent models download MODEL_ID` is also available
to prefetch one. The local runtime requires the packaged Claurst and llama.cpp files; it does not fall back to a hosted model.

`gent orchestration fanout --graph-json FILE` and `cross-review --request-json FILE` accept exact JSON `FanoutRequest`/`CrossReviewRequest` values; exact positional `/fanout FILE` and `/cross-review FILE` use the same strict inputs; `read --conversation-id ID --graph-id ID` reads their graph.
They require the explicit persistence profile's `orchestration-v1` capability, make/read only daemon-owned graph records, and never schedule or start a provider worker.
## Development

For source-built standalone Claurst run `make bootstrap-dev-claurst-runtime` to stage the digest-checked upstream Claurst and llama.cpp runtime beside debug Gentd; for Claude or Codex prompts set `GENT_NODE_BINARY` to a Node executable with sibling npm files. Published releases bundle these runtimes automatically.
To reclaim build space, run `make clean-spaces` from any directory; it anchors itself here and removes only reproducible Cargo `target/` artifacts and optional `.cargo-cache/`, never `.gentd/`, `.gent/`, `GENT_DATA_DIR`, or the shared Cargo home cache.

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo llvm-cov --workspace --all-targets --all-features --json --summary-only --output-path /tmp/gent-coverage-summary.json \
  --ignore-filename-regex '(^|/)crates/(gent-cli|gentd|gent-testkit)/|/tests/|_tests\.rs$|/src/bin/|/examples/' \
  --fail-under-lines 90
bash tools/smoke-local-ipc.sh
```

Public-driver transcript capture and the real-provider evidence gate are in
[docs/public-driver-transcript-capture.md](docs/public-driver-transcript-capture.md).

The repository’s architecture and orchestration boundary are in [docs/architecture.md](docs/architecture.md) and [the orchestration plan](docs/multi-agent-orchestration-plan.md).
The Flutter app is not a dependency of this workspace. Setup is in [docs/onboarding.md](docs/onboarding.md),
[docs/releases.md](docs/releases.md), and the [Flutter consumer handoff](docs/flutter-handoff-v1.md).

## Code architecture

`main` is the only composition root; it delegates through typed commands,
values, protocols, and ports. Product modules never reach into another domain
or infrastructure detail; the architecture check rejects direct product-domain
imports outside `gentd`. Pure transitions are testable without I/O; adapters own
edge I/O. Every hand-authored source, test, script, and CI workflow is at most
300 lines; generated lockfiles and evidence fixtures are excluded.
## Security boundary

`gentd` owns only local provider processes, model files, and configured MCP stdio declarations. Provider updates are explicit and receipt-backed; `gent doctor` only observes.

On macOS and Linux, `gentd` creates its data directory with owner-only permissions
and accepts a Unix socket only beneath that directory; the socket itself is also
owner-only. Windows uses a data-directory-derived named pipe for the same negotiated
content surface.

## License
Apache-2.0. Gent is a trademark of its respective owner; this license grants no trademark rights.
