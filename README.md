# Gent CLI

`gent-cli` is the open-source Rust host runtime for Gent. It provides the
versioned local protocol, durable command receipts and ledger used by `gentd`,
and a thin `gent` command-line client.

## Current milestone

The repository is intentionally beginning with the runtime boundary, rather
than a second copy of application logic. The implemented vertical slice is:

- `gentd`: a supervised local daemon using a Unix socket on macOS/Linux and a
  named pipe on Windows.
- `gent`: a protocol-only client that starts a local daemon on demand.
- Version negotiation, capability intersection, host-epoch fences,
  idempotent command receipts, cursor-ordered durable events, and explicit
  snapshot-backed resync after event compaction.
- SQLite-backed host state, a read-only `gent doctor` dependency report, and negotiated
  `gent conversation list`, `gent conversation status`, `gent conversation timeline`, and Unix-only
  `gent conversation content` reads. `gent conversation activity` is a protocol-only future
  authority reader; observer-mode `gentd` deliberately declines its capability.
- Explicit `gent deps` plans and consented vendor dependency actions, each fenced by the active
  host epoch and settled through a durable receipt; interrupted external effects are marked
  `unprovable` instead of being replayed.
- Durable conversation → run → turn identity and restart-safe provider-switch lineage, exposed
  only through the capability-gated read protocol in `gentd`; timeline reads exclude all message
  content and provider-native session identifiers.
- A receipt-backed user-prompt ledger that atomically assigns an active turn and retains text outside
  receipt/event payloads. The default observer exposes only cursor-bound, page-byte-bounded reads;
  an explicit local profile can durably accept create/send/queue requests, never provider output.
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

Gent owns the agent-chat runtime: durable conversations, sessions, prompts,
Claude and Codex public-driver orchestration, the private Claurst bridge behind
its port, MCP, and authorized Git work. A future Flutter integration invokes
`gent`/`gentd` through this local protocol; it never spawns a provider binary
directly. A long-lived negotiated `gentd` connection is the future app boundary;
`gent` may launch or diagnose that host, but it is not a per-prompt process
substitute for subscriptions, receipt retries, or cursor resumption. `gentd` is
the only component allowed to compose those capabilities.

The public protocol reserves strictly typed, capability-gated agent-chat reads
and intents. By default the daemon refuses mutations. For isolated local
testing, start `gentd --agent-chat-authority`: it advertises
`agent-chat-intents-v1` and durably accepts create/send/queue requests through
the same receipt and epoch checks. It still rejects provider lifecycle,
transcript streaming, MCP, Git, and private-bridge actions.

### Milestone scope contract

- `gent-cli` is the only CLI surface; it is a thin protocol client and must remain
  a composition boundary, not a hidden authority plane.
- `gentd` composes only agent-chat domains. In the current observer milestone it hard-disables
  Git mutation, MCP spawning, and private bridge routing for live provider execution.
- Modules do not import each other as peers; only `gentd` and `gent` are allowed to
  compose stable interfaces across boundaries.
- Public transport for Claude/Codex stays declarative and typed; the isolated ledger profile
  never launches either binary.
- Claurst support exists in protocol as a private-bridge capability only; no app-level credentials
  or endpoints are embedded in the public daemon.
- Device pairing and application-specific UI automations are Flutter-app responsibilities. A later
  agent-chat `gent-automations` domain is separate, port-bound, and must not subsume either concern.
- No source file in this milestone should exceed 300 lines. This is enforced by
  `python tools/check-architecture.py`.
`fixtures/ipc-contract/manifest.json` is the language-neutral compatibility
fixture for that local protocol. It records canonical JSON and the exact
four-byte big-endian length-prefixed wire frames for handshake, errors, event
resume, and reserved agent-chat values. A future Flutter client can validate
its codec against it without embedding Rust or treating any reserved capability
as available. Validate it with:

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
cargo run -p gent-cli -- doctor
cargo run -p gent-cli -- update check
cargo run -p gent-cli -- status
cargo run -p gent-cli -- --conversations
cargo run -p gent-cli -- conversation list
cargo run -p gent-cli -- submit --kind ping --payload '{"message":"hello"}'
cargo run -p gent-cli -- events
cargo run -p gent-cli -- events --follow
```

The default data directory is `~/.gentd`, separate from existing Gent/Clouseau data.
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

Running `gent` with no subcommand, or `gent --conversations`, opens the local
conversation browser. It lists durable identities and run counts. Observer mode
shows a disabled composer; the opt-in ledger profile enables input, selected
provider/model/effort/mode controls, `Ctrl+N` creation, and `Enter` prompt
persistence. `gent chat create`, `send`, and `queue` remain available for scripts;
start that isolated profile with `GENT_AGENT_CHAT_AUTHORITY=1 gentd`; none starts a provider lifecycle.

## Development

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo llvm-cov --workspace --all-targets --all-features --summary-only \
  --ignore-filename-regex '(^|/)crates/(gent-cli|gentd|gent-testkit)/|/tests/|_tests\.rs$|/src/bin/' \
  --fail-under-lines 90
bash tools/smoke-local-ipc.sh
```

## Public-driver evidence inventory

The phase-0 manifest lists every required Claude/Codex scenario without
inventing recordings. It includes redacted live `full_turn`, `tool_use`,
`tool_error`, plan-mode, and observed thinking/usage captures for Claude and
the requested Codex `gpt-5.6-luna` configuration. Four strict cells remain:
Claude persistent-permission, compaction, and malformed-tolerance; and Codex
malformed-tolerance. They are capture-required.
The coverage manifest
links to that inventory, so an authority-evidence record for Claude or Codex
must name a recorded transcript whose provider/version/platform/driver transport
agree with the record. CI
validates both manifests structurally:

```sh
cargo run -p gent-testkit --bin validate-public-driver-manifest -- fixtures/public-driver-transcripts/manifest.yml
```

To refresh public-provider fixtures after a provider/version change, run:

```sh
python3 tools/update-public-driver-transcripts.py --vendor claude
python3 tools/update-public-driver-transcripts.py --vendor codex
```

The helper prints exact provider-specific capture commands for each unrecorded
cell, including model, transport, and canonical output path. Add `--run --confirm`
only in an attended, reviewed capture session.

Use `--require-live` only at the real-provider evidence gate. It deliberately
fails until every cell is a redacted live recording or a reasoned recorded
absence; synthetic fixtures never satisfy that gate. A claimed live capture
also requires canonical executable identity and SHA-256, provider transport,
platform, RFC3339 capture time, run identifier, and attestation digest. The
capture helper's digest is a reproducible hash of reviewed redacted metadata
and normalized frames; raw provider output is bounded, discarded, and
deliberately not claimed as attested. These are structural provenance checks,
not a substitute for the planned signed real-provider artifact and
normalized-event replay gate.

Refresh an approved safe Claude/Codex cell with the redaction-first helper:

```sh
python3 tools/capture-public-driver-transcript.py claude full_turn \
  --model haiku --output fixtures/public-driver-transcripts/claude-full-turn.jsonl \
  --confirm-live-capture --update-manifest
```

It retains raw output only in memory, writes normalized facts, and refuses to
run without explicit confirmation. The native Claude subagent row uses the
separate reviewed `tools/capture-claude-subagent-transcript.py` helper: it
allows only `Task(gent_probe)` and records a correlated native `Agent` call,
matching tool result, and successful terminal event—not prompt text. Other
matrix rows need scenario-specific reviewed captures. Claurst remains
private-bridge CI evidence and is rejected by public tools, preserving the
no-credentials/no-endpoints boundary.

An observed absence can be kept as diagnostic context, but never satisfies the
real-provider `--require-live` gate: authority requires positive, redacted,
scenario-specific live capture. A parser error before a provider turn, help
output, or an unavailable flag is not provider evidence.

Codex approval, persistent-permission, plan, compaction, MCP, interrupt, and
steering scenarios use the documented app-server JSON-RPC harness rather than
one-shot `codex exec`; it has a provider-free dry run and never changes the
matrix automatically. It emits a candidate fixture only after the scenario's
correlated native protocol conditions are observed. Its MCP helper requires an
already-authenticated, isolated `CODEX_HOME` and never copies or reads credentials:

```sh
python3 tools/capture-codex-app-server-transcript.py plan_mode \
  --model gpt-5.6-luna \
  --output fixtures/public-driver-transcripts/codex-plan-mode.jsonl --dry-run
```

The repository’s architectural rules are in [docs/architecture.md](docs/architecture.md).
The Flutter app is not a dependency of this workspace. Setup is in
[docs/onboarding.md](docs/onboarding.md), [docs/releases.md](docs/releases.md), and the
[Flutter consumer handoff](docs/flutter-handoff-v1.md).

## Code architecture

`main` is the only composition root; it delegates through typed commands,
values, protocols, and ports. Product modules never reach into another domain
or infrastructure detail; the architecture check rejects direct product-domain
imports outside `gentd`. Pure transitions are testable without I/O; adapters own
edge I/O. Every hand-authored source, test, script, and CI workflow is at most
300 lines; generated lockfiles and evidence fixtures are excluded.

## Security boundary

`gentd` never receives Claurst credentials or endpoints: its bridge is
app-private and private-CI-only. Provider updates are explicit, receipt-backed;
`gent doctor` only observes. The daemon routes no live provider, MCP, Git, or
listener; its opt-in ledger is persistence only. Pairing/app automation is out.

On macOS and Linux, `gentd` creates its data directory with owner-only permissions
and accepts a Unix socket only beneath that directory; the socket itself is also
owner-only. This protects the capability-gated, locally readable user-prompt content
surface, which remains unavailable on Windows until its named-pipe ACL boundary is hardened.

## License

Apache-2.0. Gent is a trademark of its respective owner; this license grants no trademark rights.
