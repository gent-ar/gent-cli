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
  `gent conversation status` and `gent conversation timeline` reads.
- Explicit `gent deps` plans and consented vendor dependency actions, each fenced by the active
  host epoch and settled through a durable receipt; interrupted external effects are marked
  `unprovable` instead of being replayed.
- Durable conversation → run → turn identity and restart-safe provider-switch lineage, exposed
  only through the capability-gated read protocol in `gentd`; timeline reads exclude all message
  content and provider-native session identifiers.
- A dormant receipt-backed user-prompt ledger that atomically assigns an active turn and retains
  text outside receipt/event payloads; it is not yet exposed by observer-mode `gentd`.
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
directly. `gentd` is the only component allowed to compose those capabilities.

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

```sh
cargo run -p gent-cli -- doctor
cargo run -p gent-cli -- status
cargo run -p gent-cli -- submit --kind ping --payload '{"message":"hello"}'
cargo run -p gent-cli -- events
cargo run -p gent-cli -- events --follow
```

The default data directory is platform-specific. Set `GENT_DATA_DIR` to an
empty temporary directory when experimenting or testing. Set `GENTD_BIN` to a
specific daemon binary when `gent` should not resolve a sibling executable.
Pass `--no-autostart` to require an already-running daemon, which is useful for
supervised deployments and deterministic smoke tests.

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
inventing recordings. The coverage manifest links to that inventory, so an
authority-evidence record for Claude or Codex must name a recorded transcript
whose provider/version/platform/driver transport agree with the record. CI
validates both manifests structurally:

```sh
cargo run -p gent-testkit --bin validate-public-driver-manifest -- fixtures/public-driver-transcripts/manifest.yml
```

Use `--require-live` only at the real-provider evidence gate. It deliberately
fails until every cell is a redacted live recording or a reasoned recorded
absence; synthetic fixtures never satisfy that gate. A claimed live capture
also requires canonical executable identity and SHA-256, provider transport,
platform, RFC3339 capture time, run identifier, and attestation digest.
These are structural provenance checks, not a substitute for the planned
signed real-provider artifact and normalized-event replay gate.

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
adapters own I/O at the edge. Every Rust source file, including tests, is kept
at 300 lines or fewer and CI enforces that limit.

## Security boundary

`gentd` never receives Claurst credentials or endpoint configuration. Provider
installation or updates are explicit, receipt-backed user actions; `gent doctor`
only observes dependencies. The present daemon does not route or start live provider
runs, MCP servers, Git operations, or network listeners. Pairing and automation
execution are deliberately outside its protocol surface.

## License

Apache-2.0. Gent is a trademark of its respective owner; this license grants
no trademark rights.
