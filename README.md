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
  `gent conversation status --conversation-id <id>` reads.
- Durable conversation → run → turn identity and restart-safe provider-switch lineage, exposed
  only through the capability-gated read protocol in `gentd`.
- Additive, provider-neutral lifecycle signals for thinking, compacting, permission/question
  waits, subagent work, command work, and attention; these are durable status foundations, not
  a claim that a live provider is attached to the daemon.

Claude, Codex, MCP, pairing, Git, automations and the private Claurst bridge
are deliberately not routed through `gentd` in this milestone. The public
driver crate can execute a previously locked Claude or Codex binary at its
outer operating-system edge, but daemon authority is not connected to that
capability. This keeps the app as the sole production writer until the
migration plan's observer and cutover gates are satisfied.

## Try it

```sh
cargo run -p gent-cli -- doctor
cargo run -p gent-cli -- status
cargo run -p gent-cli -- submit --kind ping --payload '{"message":"hello"}'
cargo run -p gent-cli -- events
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
cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 90
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

## Code architecture

`main` is the only composition root: it wires ports, infrastructure, and the
application together, then delegates immediately. Product modules communicate
only through typed commands, value types, protocols, and exported ports. A
module never reaches into another product domain or infrastructure detail.
Pure state transitions remain small functions that can be tested without I/O;
adapters own I/O at the edge. Every Rust source file, including tests, is kept
at 300 lines or fewer and CI enforces that limit.

## Security boundary

`gentd` never receives Claurst credentials or endpoint configuration. Provider
installation is explicit and user-triggered; `gent doctor` only observes
dependencies. The present daemon does not spawn providers, MCP servers, Git,
automation jobs, or network listeners.

## License

Apache-2.0. Gent is a trademark of its respective owner; this license grants
no trademark rights.
