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
  idempotent command receipts and cursor-ordered durable events.
- SQLite-backed host state and a read-only `gent doctor` dependency report.

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
bash tools/smoke-local-ipc.sh
```

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
