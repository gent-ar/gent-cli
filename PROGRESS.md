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
- That sealed ordinary composition accepts provider grants only from one signed
  authority release after its nested evidence and compatibility checks complete.
  It derives its owner and epoch from active daemon state, accepts no evidence
  path/key or client-selected owner, and can compose Codex without unavailable
  Claude evidence. Its pure gate admits only Claude/Codex Ask or Plan selections.
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
  settlement authority. The obsolete generic provisioning owner was removed;
  only the prompt-scoped path may later verify/install-lock through the private
  Node prefix and atomically settle its exact receipt. Denied consent and plan
  mismatch start no npm, and a recovered accepted receipt becomes unprovable.
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
- Every `SendPrompt` is held durably as `awaiting_readiness`. A daemon-only
  ready fact binds its prompt receipt, conversation, current run, provider, and
  epoch; one immediate SQLite transaction stores that fact, settles its exact
  idempotency receipt, and releases only that held prompt. A changed selection,
  event collision, stale epoch, missing lock, or unreadable provenance exposes
  no work. This is fresh-schema storage, not a migration or recovery snapshot.
- An explicit post-commit readiness-admission seam checks the durable locked
  Claude/Codex executable, releases only after the preceding transaction, then
  notifies a downstream lifecycle. It launches nothing inline; missing or
  invalid installations remain held. Generic and observer chat persistence do
  not opt into this wake path.
- `provider-readiness-v2` carries only exact conversation/run identity and
  returns Ready, a daemon-generated install review, or a safe unavailable
  reason. The terminal now has capability-gated `gent provider readiness` and
  `gent provider provision` commands. It can only correlate a held prompt,
  consent, and the daemon-issued digest; it never sends a provider, package,
  command, executable, policy, or plan. Provision retries deterministically
  reuse their receipt for the same idempotency key. Observer remains absent.
- The unadvertised prompt-provider-provision-v1 command fingerprint binds the
  daemon-selected package coordinates and policy digest, rechecks the exact
  signed selection before npm, and atomically settles verified provenance plus
  its prompt release. Consent refusal, digest mismatch, and proven pre-effect
  failure keep the prompt held; ambiguity is durable `unprovable` with no replay.
- Conversation detail now exposes the durable current run identity explicitly,
  rather than asking either client to infer it from a run list. That identity is
  the selection token a future readiness review and fenced prompt will share.
- The obsolete path/key ordinary-authority input parser was removed rather than
  retained as a second source of truth. A future explicit bootstrap must load
  the one signed authority release against the locked Node runtime and reuse
  only its verified grants; it is not yet a daemon argument or transport entry.
- A bounded, read-only signed package-policy release artifact admits only the
  official Claude/Codex package identities, exact semantic versions, canonical
  SHA-512 tarball integrity, expiry/revocation, and the current locked Node
  digest. It is revalidated before use and is not a prompt-time cache, writer,
  or observer capability.
- The app-supplied Node lock also pins npm's CLI module. Fixed package commands
  run that module through the exact locked Node binary and recheck the full
  Node/npm/CLI chain before each npm effect, never resolving host Node through
  `PATH`. This remains dormant in observer mode.
- The locked Node child environment now replaces inherited `PATH` with the
  locked Node directory plus required system interpreters and removes inherited
  Node/npm configuration. It is shared by npm pack/install, ordinary shims, and
  the fixed provider `--version` verifier, which now runs through the same
  lock-rechecked bounded launcher rather than an ambient executable invocation.
- The dormant ordinary Claude/Codex Ask/Plan composition now creates only a
  lock-rechecked app-Node launcher, so npm-installed shims cannot select an
  ambient Node runtime. Agent, Autonomous, and Bypass remain unavailable.
- One bounded, strict signed ordinary-authority release artifact now embeds its
  selected public providers, compatibility envelope, evidence, package policy,
  and delegated verification keys. It validates every inner signature and binds
  package policy to the locked Node before returning material. Its canonical
  complete-artifact SHA-256 is retained for the future receipt/provenance fence;
  no bootstrap reads it yet and no independent authority paths are accepted.
  It rejects duplicate provider grants, and uses the existing protected runtime-
  update Ed25519 release root, not a new signing setup; runtime metadata and
  provider authority remain distinct data.
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

1. Complete the prompt-scoped provisioning authority: durably settle consent/plan refusal, derive
   the reviewed package command from the current run, and compose its capability only after
   strict provider evidence and sandbox proof.
2. Prove terminal follow, context/provider switching, `/goal`, backpressure,
   process-tree drain, terminal settlement, and reconnect by durable cursors.
3. Add the private Claurst bridge under the identical public fact contract and
   its private CI evidence; then complete native-app IPC parity and remove app
   drivers in a separately authorized clean cutover.

Nothing in this file claims live provider execution, app cutover, or release
readiness until those gates have direct evidence.
