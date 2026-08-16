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
  receipt/event payloads. Unix-only content reads are cursor-bound, page-byte-bounded, and limited
  to user-authored prompt text; no provider output is exposed in this observer milestone.
- Durable workspace → repository → worktree identities, stored independently from worktree
  leases and any future Git execution.
- Durable worktree-scoped Git-operation lifecycle records with optimistic, terminal-safe transitions;
  recording an operation never starts a Git process.
- Durable credential-free tool-source declarations for MCP, built-in, and host integrations;
  declaring one neither connects to nor starts a tool source.
- Durable workspace-scoped automation-execution records with trigger deduplication and terminal-safe
  transitions; recording one neither evaluates a schedule nor starts automation work.
- Durable ordered run checkpoints with monotonic event cursors and SHA-256 state references;
  checkpoint records never contain opaque provider state or resume a live process.
- Versioned, append-only provider-permission policy records with canonical allow-lists; they
  intentionally exclude credentials, provider endpoints, and bridge configuration.
- Additive, provider-neutral lifecycle signals for thinking, compacting, permission/question
  waits, subagent work, command work, and attention; these are durable status foundations, not
  a claim that a live provider is attached to the daemon. Root generation activity is explicit,
  so waiting on detached work is never inferred from a root turn phase alone.

### Intended product boundary (not wired in this observer milestone)

Gent owns the agent-chat runtime: durable conversations, sessions, prompts,
Claude and Codex public-driver orchestration, the private Claurst bridge behind
its port, MCP, and authorized Git work. A future Flutter integration invokes
`gent`/`gentd` through this local protocol; it never spawns a provider binary
directly. A long-lived negotiated `gentd` connection is the future app boundary;
`gent` may launch or diagnose that host, but it is not a per-prompt process
substitute for subscriptions, receipt retries, or cursor resumption. `gentd` is
the only component allowed to compose those capabilities.

The public protocol already reserves strictly typed, capability-gated
agent-chat conversation/transcript reads and future create, send, queue,
interrupt, decision, and cursor-subscription intents. Those frames are an
uncomposed compatibility contract: negotiating them does not enable a provider,
write a conversation, or change the observer daemon's refusal behavior.

`fixtures/ipc-contract/manifest.json` is the language-neutral compatibility
fixture for that local protocol. It records canonical JSON and the exact
four-byte big-endian length-prefixed wire frames for handshake, errors, event
resume, and reserved agent-chat values. A future Flutter client can validate
its codec against it without embedding Rust or treating any reserved capability
as available. Validate it with:

```sh
cargo run -p gent-testkit --bin validate-ipc-fixtures -- fixtures/ipc-contract/manifest.json
```

Device pairing and application automations are Flutter-app concerns, not a
`gentd` API or execution domain. The workspace retains small pure policy/value
crates needed by the platform contract, but neither is wired into the daemon or
available through the CLI protocol.

Claude, Codex, MCP, Git, and the private Claurst bridge are deliberately not
routed through `gentd` in the current observer milestone. The public driver
crate has minimal typed launch specifications for a previously locked Claude
or Codex binary at its operating-system edge, but daemon authority is not yet
connected to that capability. This keeps the app as the sole production writer
until the migration plan's evidence, observer, and cutover gates are satisfied.

## Try it

Install a signed release (or explicitly rerun with `--force` to update it).
This is a paired, manual installer update, not an automatic daemon self-update.
Choose a published tag, download the bootstrap asset and its Sigstore bundle,
and verify the bootstrap before executing it:

```sh
version=vX.Y.Z
curl -fLO "https://github.com/gent-ar/gent-cli/releases/download/$version/gent-install.sh"
curl -fLO "https://github.com/gent-ar/gent-cli/releases/download/$version/gent-install.sh.sigstore.json"
cosign verify-blob gent-install.sh --bundle gent-install.sh.sigstore.json \
  --certificate-identity-regexp "^https://github.com/gent-ar/gent-cli/.github/workflows/release.yml@refs/tags/$version$" \
  --certificate-oidc-issuer https://github.com/login/oauth
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
cosign verify-blob gent-install.ps1 --bundle gent-install.ps1.sigstore.json --certificate-identity-regexp "^https://github.com/gent-ar/gent-cli/.github/workflows/release.yml@refs/tags/$version$" --certificate-oidc-issuer https://github.com/login/oauth
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

The default data directory is platform-specific. Set `GENT_DATA_DIR` to an
empty temporary directory when experimenting or testing. Set `GENTD_BIN` to a
specific daemon binary when `gent` should not resolve a sibling executable.
Pass `--no-autostart` to require an already-running daemon, which is useful for
supervised deployments and deterministic smoke tests.

`gent update check` is a truthful read-only status probe: this observer build
has no configured runtime-release metadata source, so it reports
`releaseMetadataUnavailable`. To update from an already reviewed release, use
the explicit external handoff below. It verifies the tag-bound installer
bootstrap with Sigstore, and that installer independently verifies the archive,
manifest, and supplied archive digest before staging the immutable binary pair.
It refuses
to switch the pair while `gentd` owns the selected data directory:

```sh
digest='target archive digest from the signed manifest'
gent --data-dir "$GENT_DATA_DIR" update apply \
  --version vX.Y.Z \
  --expected-sha256 "$digest" \
  --consent
```

Pass `--install-dir DIR` when the runtime is not installed under the default
root. The command never selects `latest`, starts or replaces `gentd`, or
silently falls back to another archive. Stop the target daemon first; after a
successful handoff, start `gent` normally to launch the selected pair.

Running `gent` with no subcommand, or `gent --conversations`, opens the local
read-only conversation browser. It lists durable identities and run counts, then
shows an explicitly disabled composer and model/effort/mode controls while the
daemon is in observer mode. It never sends a prompt, a command receipt, or a
provider lifecycle request.

## Development

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo llvm-cov --workspace --all-targets --all-features --summary-only \
  --ignore-filename-regex '(^|/)(gentd|gent-testkit)/|/tests/|_tests\.rs$|/src/bin/' \
  --fail-under-lines 90
bash tools/smoke-local-ipc.sh
```

## Public-driver evidence inventory

The phase-0 manifest lists every required Claude/Codex scenario without
inventing recordings. It includes redacted live `full_turn`, `tool_use`,
`tool_error`, plan-mode, and observed thinking/usage captures for Claude and the
requested Codex `gpt-5.6-luna`
configuration; every unexercised scenario remains explicitly capture-required.
The coverage manifest
links to that inventory, so an authority-evidence record for Claude or Codex
must name a recorded transcript whose provider/version/platform/driver transport
agree with the record. CI
validates both manifests structurally:

```sh
cargo run -p gent-testkit --bin validate-public-driver-manifest -- fixtures/public-driver-transcripts/manifest.yml
```

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
correlated native protocol conditions are observed. Review it before manually
updating the manifest:

```sh
python3 tools/capture-codex-app-server-transcript.py plan_mode \
  --model gpt-5.6-luna \
  --output fixtures/public-driver-transcripts/codex-plan-mode.jsonl --dry-run
```

The repository’s architectural rules and phased migration decision record are
in [docs/architecture.md](docs/architecture.md). The Flutter app is not a
dependency of this workspace and is not modified by this repository.

Standalone setup and signed-release verification are documented in
[docs/onboarding.md](docs/onboarding.md) and [docs/releases.md](docs/releases.md).

## Code architecture

`main` is the only composition root: it wires ports, infrastructure, and the
application together, then delegates immediately. Product modules communicate
only through typed commands, value types, protocols, and exported ports. A
module never reaches into another product domain or infrastructure detail.
The architecture check rejects direct product-domain imports in every production
module except the `gentd` composition root.
Pure state transitions remain small functions that can be tested without I/O;
adapters own I/O at the edge. Every hand-authored Rust source, test, script,
and CI workflow file is kept at 300 lines or fewer and CI enforces that limit.
Generated lockfiles and recorded evidence fixtures are excluded from this
source-size rule.

## Security boundary

`gentd` never receives Claurst credentials or endpoint configuration. Provider
installation or updates are explicit, receipt-backed user actions; `gent doctor`
only observes dependencies. The present daemon does not route or start live provider
runs, MCP servers, Git operations, or network listeners. Pairing and automation
execution are deliberately outside its protocol surface.

On macOS and Linux, `gentd` creates its data directory with owner-only permissions
and accepts a Unix socket only beneath that directory; the socket itself is also
owner-only. This protects the capability-gated, locally readable user-prompt content
surface, which remains unavailable on Windows until its named-pipe ACL boundary is hardened.

## License

Apache-2.0. Gent is a trademark of its respective owner; this license grants
no trademark rights.
