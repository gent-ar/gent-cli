# Architecture and integration boundary

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

Permission evaluation is a pure `gent-core` function over a typed request and an immutable policy
revision. The `permission-policy-v1` local IPC extension only reads or appends those secret-free
settings. It never launches a provider; Plan permits reads only. One explicit terminal and daemon
confirmation persists Bypass without a launch flag; Autonomous/Bypass fail closed without OS containment.

The agent-chat domains are `gent-adapters`, `gent-drivers`, `gent-git`,
`gent-mcp`, the private external-provider bridge port, and the durable
conversation/session runtime. In the future authority profile, only `gentd`
composes them; callers, including a later Flutter integration, invoke `gent`
or the local protocol rather than launching Claude, Codex, Claurst, or MCP
processes directly.

Device pairing and application-specific UI automations are explicitly
Flutter-app-owned. They are outside this workspace: they are not `gentd` APIs
and have no CLI, persistence, or daemon composition path here. A later
agent-chat `gent-automations` domain remains distinct, port-bound, and cannot
obtain authority over those app-owned concerns.

This is the standalone repository's current product-scope decision. It narrows
older planning material that provisionally grouped pairing and automations with
daemon-owned domains; it does not change that source material in the Flutter
application repository.

The zero-user/single-developer standalone path has no legacy app or fleet to
migrate, so it does not require a deployed fence-aware legacy release. This
project still does not replace Flutter behavior. Before a future Flutter launch
uses Gent, it must establish protocol compatibility and exactly one active
writer/host epoch; no client may bypass that guard.

The current daemon hard-disables public provider lifecycle work in observer
mode. Existing legacy-tap utilities are compatibility experiments, not a
required migration or an authority-transfer claim.

## Future lifecycle and runtime-update boundary

When authority is separately approved, `gentd` will be the sole writer of a
versioned `ConversationActivity` projection for agent-chat conversations. The
projection will be cursor- and revision-bound, derive thinking, command,
subagent, decision, interruption, and terminal states only from durable
facts, and be the client fallback for a compatible Flutter caller. A client
may render that projection or show transport staleness; it must not infer a
lifecycle from provider text or timers.

The existing run-level lifecycle reducers are foundations, not this public
projection. The default observer daemon offers no authoritative provider
lifecycle and the terminal client remains content-free with its composer
disabled. The explicit `--agent-chat-authority` profile is narrower: it writes
only durable create/send/queue intent records under the usual fences, and never
composes a provider, MCP, Git, or private bridge. No client can bypass receipts,
cursor resume, epoch fencing, or capability negotiation.

The observer daemon does not advertise runtime-update work. An explicit
`--runtime-update-check-authority` profile may advertise only the metadata-only
check contract after it loads a locally cached, signed release with a trusted
public key; the cache is revalidated on every request. It never fetches a tag,
writes a ledger checkpoint, downloads an archive, or activates a runtime.
Separately, `gent update apply` requires a tag, exact target digest, and
`--consent`. Its client verifies a tag-bound signed bootstrap; the installer
independently verifies the archive/manifest, stages `gent`/`gentd` together,
and takes the daemon host lock during the atomic pointer switch. A signed
external Unix supervisor health-checks a staged pair, waits for idle, and rolls
back on successor-health failure. It never replaces a live daemon in process.

The installed macOS/Linux pair also contains a signed external updater. Opt-in
`gent update auto` registers a user LaunchAgent or systemd-user timer. GitHub
`latest` is untrusted discovery only; every selected tag repeats bootstrap and
archive verification and uses the same idle-only supervisor path. The helper
serializes runs and records bounded retry backoff. Windows currently offers the
signed manual pair only. This is distribution, not daemon-update authority:
observer `gentd` never fetches, schedules, stages, or activates itself.

A future live daemon-update authority must still validate protocol/schema/app
compatibility, retain rollback, and leave ingress closed on incompatible or
unhealthy successors. It needs its own authority and update-under-load evidence.

After an external supervisor has staged and started the paired successor, its
explicit `gentd --runtime-update-recover-authority` profile can revalidate the
signed cached release, durable receipt, release identity, and closed old epoch.
Only then does it confirm the durable handoff. It binds local IPC while ingress
remains closed, then atomically fences/opens the new epoch. It does not fetch,
stage, launch, or replace a process; the default daemon never enables this profile.

Device pairing, LAN transport, relay hosting, and application-specific UI
automations stay Flutter-app-owned. They do not grant a second coordinator or
become `gentd` APIs; any future app transport consumes the same negotiated
projection and receipt/cursor protocol rather than duplicating lifecycle
inference. A future `gent-automations` agent domain is separate from that app
scope and must establish its own port, receipt, evidence, and authority gates.

## Verification scope

The 90% line-coverage gate applies to production library source. The `gentd`
and `gent` binaries are composition roots and `gent-testkit` is test support;
they are covered by full workspace tests, local-IPC smoke tests, and the
platform matrix rather than included as uninstrumented source in the library
coverage denominator.
