# Architecture and migration boundary

This workspace implements the runtime-first portion of the Gent platform
contract. `gentd` is the only future writer for the Gent ledger and `gent` is
a protocol client: it must never link the store or spawn a provider directly.

The public crate dependency rules are encoded in the workspace layout:

| Crate | Role |
| --- | --- |
| `gent-types` | Stable value types and error taxonomy |
| `gent-ports` | Provider and external bridge boundaries |
| `gent-core` | Receipt, epoch and reducer rules; no I/O |
| `gent-protocol` | Versioned wire DTOs and negotiation |
| `gent-store` | SQLite persistence |
| `gent-runtime` | Coordinator implementation |
| `gentd` | Composition and local IPC server |
| `gent-cli` | Protocol-only command-line client |

The remaining domain crates (`gent-adapters`, `gent-drivers`, `gent-git`,
`gent-mcp`, `gent-automations`, `gent-pairing`, and `gent-testkit`) exist as
explicit ownership boundaries. They cannot obtain write authority or launch
external work in this milestone.

No migration authority transfers to this repository until recorded baseline
transcripts, observer parity, public-driver evidence, app compatibility and
the fence-aware legacy release all pass. In particular, this project does not
currently replace any Flutter application behavior.

## Verification scope

The 90% line-coverage gate applies to production library source. The `gentd`
and `gent` binaries are composition roots and `gent-testkit` is test support;
they are covered by full workspace tests, local-IPC smoke tests, and the
platform matrix rather than included as uninstrumented source in the library
coverage denominator.
