# Gent CLI progress

This is a concise, evidence-bound progress record for the current Gent worktree.
It distinguishes implemented foundations from the still-uncomposed live agent
authority. For detailed contracts, read `docs/continuation-handoff.md` and
`docs/realtime-agent-chat-client-plan.md`.

## Product direction

Gent is the sole harness and durable source of truth. The terminal and native
app are equal local-IPC clients of one demand-started, multiplexed `gentd` per
private `.gentd` profile. The host owns its one ledger, provider queue, shared
resource limits, process trees, and lifecycle facts. It remains while it has
active work, pending decisions, or clients, and may stop under an explicit idle
policy. The app must not spawn a `gent` or provider process per conversation.

Conversation creation now canonicalizes its raw local path only at the daemon
boundary and atomically binds the derived workspace record to its root run in
the ledger. Before a live launch, that binding must still be revalidated and
passed only to the matching provider; neither a client prompt nor daemon cwd
may become a source of truth. On reconnect, clients reread bounded durable
pages and resume by cursor. Snapshots, recovery snapshots, caches, and mirrored
lifecycle state are prohibited.

## Implemented and verified foundations

- Typed local IPC, receipt/idempotency/epoch fences, SQLite ledger, immutable
  run/turn lineage, bounded cursor pages, and normalized durable fact seams.
- Durable conversation selection, prompt persistence, goals, reviewed-plan
  artifact/approval reservation, clear-context ordinal-zero boundaries, and
  provider-switch child runs.
- A committed prompt acceptance carries the exact durable conversation, run,
  and turn IDs. `gent <prompt>` defaults a newly created selection to Ask.
- `gent chat create` and a new direct prompt bind the terminal current directory
  (or explicit `--workspace`) to one daemon-canonical workspace in the same
  SQLite transaction as the conversation/root run and receipt. Unbound ledger
  fixtures cannot accept prompts.
- A private ordinary-lifecycle router resolves a committed prompt's provider
  from the durable run selection and arms only that bounded host. It retains no
  durable state and is injected only by a dormant ordinary-authority facade
  constructor, never by default observer composition.
- Pure Claude/Codex normalizers, locked-process/session runner seams, bounded
  output/backpressure/drain primitives, and private Claurst bridge port types.
- Opt-in, redacted development transcript corpus and validation tooling; it is
  not a runtime recorder, lifecycle authority, or substitute for live evidence.
- No public Claurst credential, endpoint, or routing implementation exists.

## Not complete / not advertised

- Default `gentd` is hard observer. The existing agent-chat authority persists
  intent only; it cannot launch a provider, and ordinary Claude/Codex authority
  is still uncomposed.
- No client currently receives a live Claude, Codex, or Claurst execution path.
  The private Claurst bridge requires its private implementation and CI evidence.
- Four strict public evidence cells remain: Claude persistent permission,
  Claude compaction, Claude malformed tolerance, and Codex malformed tolerance.
  No recording may be fabricated.
- Autonomous/Bypass, live login, provisioning, reviewed-plan execution,
  multi-agent dispatch, and native-app driver removal remain authority/evidence
  gated. Their contracts do not authorize hidden fallback launchers.
- The enforced workspace coverage threshold is 90%; the recorded local result
  is 90.69%, below the requested 100% coverage target.

## Current implementation path

1. Compose one explicit ordinary Ask/Plan Claude/Codex authority profile behind
   `gentd`, retaining hard observer as the default.
2. Connect prompt-commit wakeups to a bounded daemon lifecycle router that
   resolves the canonical run/workspace, rechecks locks/policy/evidence, and
   produces normalized facts before client broadcast.
3. Prove terminal follow, context/provider switching, `/goal`, backpressure,
   process-tree drain, terminal settlement, and reconnect by durable cursors.
4. Add the private Claurst bridge under the identical public fact contract and
   its private CI evidence; then complete native-app IPC parity and remove app
   drivers in a separately authorized clean cutover.

Nothing in this file claims live provider execution, app cutover, or release
readiness until those gates have direct evidence.
