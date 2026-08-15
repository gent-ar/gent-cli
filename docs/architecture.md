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

Driver-to-runtime conversion is composed only at the `gentd` edge. `gent-runtime` receives
protocol, type, and port values rather than importing another product domain.

The agent-chat domains are `gent-adapters`, `gent-drivers`, `gent-git`,
`gent-mcp`, the private external-provider bridge port, and the durable
conversation/session runtime. In the future authority profile, only `gentd`
composes them; callers, including a later Flutter integration, invoke `gent`
or the local protocol rather than launching Claude, Codex, Claurst, or MCP
processes directly.

`gent-automations` and `gent-pairing` retain pure platform-contract policy and
value boundaries, but device pairing and application automations are explicitly
Flutter-app-owned. They are not `gentd` APIs, are not executable CLI domains,
and will not receive a daemon composition path. No domain crate can obtain
write authority or launch external work in this milestone.

This is the standalone repository's current product-scope decision. It narrows
older planning material that provisionally grouped pairing and automations with
daemon-owned domains; it does not change that source material in the Flutter
application repository.

No migration authority transfers to this repository until recorded baseline
transcripts, observer parity, public-driver evidence, app compatibility and
the fence-aware legacy release all pass. In particular, this project does not
currently replace any Flutter application behavior.

The current daemon hard-disables public provider lifecycle work in observer
mode, but that is not the phase-4 legacy observer profile. That later profile
must consume a legacy event tap without opening a Rust ledger, exposing a
mutation API, or acquiring a worktree lease.

## Future lifecycle and runtime-update boundary

When authority is separately approved, `gentd` will be the sole writer of a
versioned `ConversationActivity` projection for agent-chat conversations. The
projection will be cursor- and revision-bound, derive thinking, command,
subagent, decision, interruption, and terminal states only from durable
facts, and be the client fallback for a compatible Flutter caller. A client
may render that projection or show transport staleness; it must not infer a
lifecycle from provider text or timers.

The existing run-level lifecycle reducers are foundations, not this public
projection. Observer-mode `gentd` currently offers no authoritative provider
lifecycle and the terminal client must remain content-free with its composer
disabled. No client can use the future projection to bypass receipts, cursor
resume, epoch fencing, or capability negotiation.

The standalone release design may later let a compatible `gentd` self-update
without rebuilding an app: only a signed, digest-verified, protocol/schema/
app-range-compatible artifact may be staged; activation must drain or durably
hand off work, pass a local health handshake and read-only probe, and retain a
safe rollback path. A forward-only migration, revoked build, failed health
probe, or incompatible app range must instead leave ingress closed in a clear
read-only/update-required state. This is deliberately not implemented or
advertised by the current observer daemon.

Device pairing, LAN transport, relay hosting, and application automations stay
Flutter-app-owned. They do not grant a second coordinator or become `gentd`
APIs; any future app transport consumes the same negotiated projection and
receipt/cursor protocol rather than duplicating lifecycle inference.

## Verification scope

The 90% line-coverage gate applies to production library source. The `gentd`
and `gent` binaries are composition roots and `gent-testkit` is test support;
they are covered by full workspace tests, local-IPC smoke tests, and the
platform matrix rather than included as uninstrumented source in the library
coverage denominator.
