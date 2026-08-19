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
  and turn IDs. `gent <prompt>` defaults a newly created selection to Ask and
  automatically follows a turn only when the daemon explicitly negotiates its
  durable turn-follow capability; observer mode remains acceptance-only.
- `gent chat create` and a new direct prompt bind the terminal current directory
  (or explicit `--workspace`) to one daemon-canonical workspace in the same
  SQLite transaction as the conversation/root run and receipt. Unbound ledger
  fixtures cannot accept prompts.
- Dormant Codex and Claude lifecycle starts now resolve that workspace per run,
  pass it to the provider and process current directory, and derive their
  sandbox profile from the same root and durable Ask/Plan/Agent access mode.
  Private authority configuration is path-free, preventing one conversation's
  workspace from leaking into another on a multiplexed daemon.
- A private ordinary-lifecycle router resolves a committed prompt's provider
  from the durable run selection and arms only that bounded host. It retains no
  durable state and is injected only by a dormant ordinary-authority facade
  constructor, never by default observer composition.
- Bounded provider hosts now disarm once recovery, active-turn polling, and
  shutdown draining are complete. A later committed prompt re-arms only its
  selected host, preventing an idle settled session from creating a permanent
  polling cadence.
- That sealed ordinary composition preflights both private evidence records
  before host construction. Its pure gate accepts any valid model/effort for
  Claude/Codex Ask or Plan, while rejecting every other provider or mode before
  a ledger write; executable compatibility remains the lock-checked launch gate.
- Its one daemon-owned, notification-driven cadence is paired with the
  post-commit prompt wake. It replays durable recovery work at composition
  startup and polls only while a bounded host reports active work; it retains no
  snapshot, cache, or mirrored lifecycle state and is still not bootstrapped.
- Private ordinary hosts reject shutdown before recovery, while recovered idle
  hosts queue only their drain command—never a synthetic prompt wake. The
  router aggregates shutdown/escalation/completion only after an owner-proven
  terminal drain; an undrained tree stays failed closed. Separate transient IPC cancellation stops accepts and gracefully closes
  negotiated connections without a task abort or ledger side effect.
- The dormant ordinary cadence now begins closed, opens only after durable
  recovery reaches idle, and exits unopened on an earlier shutdown request. Its
  sealed facade ingress holds one transient permit from before prompt
  persistence through the post-commit wake; shutdown closes admission then
  waits for existing permits before draining. These controls are not durable
  state and are not composed by the observer daemon.
- Its dormant Ask/Plan path now uses the bounded, lock-rechecked direct-host
  launcher restricted to read-only workspace access. This does not relax the
  separate enforced-sandbox requirement for Agent, Autonomous, or Bypass work.
- One typed runtime capability profile now drives daemon service composition and
  wire advertisement. Turn following and reviewed plans require their explicit
  profile features (and agent chat); observer and persistence-only profiles
  stay absent without reverse-inferring authority from wire strings.
- Pure Claude/Codex normalizers, locked-process/session runner seams, bounded
  output/backpressure/drain primitives, and private Claurst bridge port types.
- Codex app-server handshake, thread, and turn frames now always include the
  required JSON-RPC 2.0 marker, and its session supports documented cooperative
  interruption. The unused generic external-bridge protocol was removed;
  `PrivateClaurstBridge` is the only private bridge contract.
- Opt-in, redacted development transcript corpus and validation tooling; it is
  not a runtime recorder, lifecycle authority, or substitute for live evidence.
- Once a private provider prefix is composed, dependency discovery resolves
  only its locked `bin` entries and never falls back to a host `PATH` CLI.
- The dormant private provisioner now rechecks the complete canonical
  dependency-action command before npm: receipt/idempotency/epoch plus provider,
  action, consent, and reviewed-plan digest.
- The fixed private npm install path disables lifecycle scripts during both
  tarball packing and verified archive installation; no package `preinstall`,
  `install`, or `postinstall` hook may run from this path.
- Verified private installations now have append-only fresh-schema provenance:
  accepted receipt/idempotency/epoch, immutable executable lock, exact package
  name/version/integrity, signed-policy digest, and supplied Node digest. The
  verified policy itself supplies its digest; a prompt or caller cannot do so.
- The private settlement coordinator atomically records that installation and
  terminally settles the receipt; ambiguous npm effects become `Unprovable` and
  cannot replay. Claude/Codex dormant resolution reads only that lock and
  rechecks its exact identity—there is no prefix or `PATH` rediscovery.
- Shared receipt reservation now makes the restart rule reusable without sharing
  settlement authority. The dormant private provisioning authority alone claims
  a daemon-issued plan, verifies/install-locks through the private Node prefix,
  and atomically settles that exact receipt; denied consent and plan mismatch
  start no npm, and a recovered accepted receipt becomes unprovable.
- Private provider verification now emits an explicitly unbound observed lock.
  A typed signed-compatibility binder must revalidate and bind its exact
  provider/version/digest at operation time before settlement; expiry or a
  mismatch becomes unprovable with no durable runnable lock. This remains
  private composition, not an observer capability or bootstrap path.
- Dormant Claude and Codex composition now reauthorizes each durable executable
  lock against the current signed compatibility window immediately before every
  provider effect, including a resumed session. An expired or revoked lock is
  refused before the runner is invoked; this current-time check is absent from
  observer composition and does not add a snapshot or cache.
- A separate private readiness service now checks only durable Claude/Codex
  locks and their current filesystem identity. It returns a daemon-generated
  install review for missing or changed locks, fails closed when provenance is
  unreadable, and never touches Node, npm, a prompt, or Claurst. It remains
  uncomposed until the reviewed-consent and selection gates are proven.
- Provider-ready prompt admission now has an atomic exact-run fence: a private
  caller can require the reviewed run to still be current in the same SQLite
  transaction that writes the prompt. A changed selection writes nothing.
  Every new `SendPrompt` is instead held durably as `awaiting_readiness`; only
  an internal, epoch- and current-run-fenced release can make it claimable.
  This is a fresh-schema revision, not a migration or recovery snapshot.
- Accepted send receipts now report `awaitingReadiness`, rather than implying a
  provider outbox entry. The generic chat path does not wake a lifecycle for
  that state; only a future private readiness authority can release the held
  prompt and then issue its lifecycle wake.
- provider-readiness-v1 now carries only an exact conversation/run assessment.
  Its explicit profile derives Ready, a daemon-generated install review, or a
  safe unavailable reason from durable Gent facts; clients cannot inject a
  provider, executable, lock, or plan. Observer and chat-only profiles do not
  advertise it, and it has no composed provision-confirmation action.
- The unadvertised prompt-provider-provision-v1 contract accepts only receipt, held-prompt,
  conversation/run, consent, epoch, and the daemon review digest. It cannot carry provider,
  package, executable, policy, or plan fields. Its SQLite settlement writes the verified lock,
  terminal provision receipt, and release of that exact held send prompt in one immediate
  transaction; a separate immediate admission changes only that exact dispatch to
  `provisioning`, blocking a competing selection switch until terminal settlement. The
  corresponding capability has no transport or bootstrap composition yet.
- Conversation detail now exposes the durable current run identity explicitly,
  rather than asking either client to infer it from a run list. That identity is
  the selection token a future readiness review and fenced prompt will share.
- A sealed all-or-nothing ordinary-authority input parser rejects partial
  evidence/compatibility settings and durable-chat-profile conflicts without
  I/O. It accepts no coordinator or epoch and is not yet a daemon argument or
  transport entry point.
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
- A bounded live Codex 0.144.1 plan-mode probe was captured on 2026-08-19 and
  recorded as a reviewed development transcript. It is intentionally not
  admitted as authority evidence or used to replace the manifest's established
  compatibility record.
- Autonomous/Bypass, live login, provisioning, reviewed-plan execution,
  multi-agent dispatch, and native-app driver removal remain authority/evidence
  gated. Their contracts do not authorize hidden fallback launchers.
- The repaired coverage gate remains 90% while focused missing-line tests are
  added; a canonical full-workspace measurement is still required before any
  claim toward the requested 100% target.

## Current implementation path

1. Complete the prompt-scoped provisioning authority: derive the reviewed plan from the current
   run, reserve/re-read its daemon-built command immediately before npm, then compose its
   capability only after strict provider evidence and sandbox proof.
2. Prove terminal follow, context/provider switching, `/goal`, backpressure,
   process-tree drain, terminal settlement, and reconnect by durable cursors.
3. Add the private Claurst bridge under the identical public fact contract and
   its private CI evidence; then complete native-app IPC parity and remove app
   drivers in a separately authorized clean cutover.

Nothing in this file claims live provider execution, app cutover, or release
readiness until those gates have direct evidence.
