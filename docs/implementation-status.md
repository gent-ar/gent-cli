# Implementation Status

This document records implemented repository work separately from live-runtime
evidence. A checked box means code and deterministic tests exist here; it does
not claim provider or app compatibility evidence that has not been recorded.
For the current working-tree context and continuation order, read [the handoff](continuation-handoff.md).

## Implemented foundations

- [x] Thirteen-crate Rust workspace with enforced dependency law.
- [x] Protocol negotiation and bounded length-prefixed JSON local IPC.
- [x] SQLite receipt/event ledger, idempotency, event cursors, and epoch checks.
- [x] Explicit public dependency actions are daemon-owned, plan-digest reviewed,
      epoch-fenced, receipt-backed, and never replay an ambiguous accepted external effect.
- [x] Durable local attachment staging with per-transfer opaque staging keys, fenced progress,
      and receipt-bound follow-up operations,
      retry-safe final content promotion, and content-addressed deduplication; it is not exposed
      to providers through observer-mode `gentd` and does not imply provider attachment support.
- [x] Capability-gated local attachment IPC with typed transfer identity, base64 chunk validation,
      retry-safe commit, durable resume, and independently idempotent begin/append/commit
      receipts. The shared receipt journal fingerprints each mutation command and records an
      accepted and terminal event before a retry is acknowledged.
- [x] Durable event snapshots and transactional compaction with explicit stale-cursor resync.
- [x] Versioned, checksummed, transactional SQLite migrations with legacy-ledger upgrade tests.
- [x] Durable run and worktree lease arbitration with separate-connection contention tests.
- [x] File-backed SQLite restart recovery for host epoch and cursor-ordered receipt events.
- [x] Pure run-lineage, cursor-deduplicated lifecycle projection, and live-status reducers.
- [x] Pure, content-safe legacy lifecycle shadow comparator and a read-only legacy-tap port;
      it is not composed into `gentd` and has no legacy-host or provider evidence yet.
- [x] Ephemeral read-only legacy-tap polling service that blocks further projection advancement
      at the first divergence; it owns no SQLite ledger, IPC mutation surface, or process work.
- [x] Pure idempotent decision-settlement reducer with unprovable and recovery-required terminal paths.
- [x] Durable SQLite decision settlement with restart-safe terminal outcomes and optimistic contention handling.
- [x] Public clients can submit a decision or explicitly mark recovery outcomes, but cannot assert
      provider acknowledgement or settlement; those lifecycle facts are accepted only behind a
      daemon-owned ingress, with legacy wire evidence rejected after negotiation.
- [x] Dormant authority-gated provider-effect ingress persists a stable, secret-free source event
      before daemon-owned session binding, decision settlement, or run projection reduction; it
      rejects source-ID substitution and all observer-mode calls, and is not composed by `gentd`.
- [x] A dormant `gentd` composition-edge adapter maps validated public-driver session effects
      into that daemon-owned ingress; process-local retry effects are never persisted and
      observer mode remains denied.
- [x] Protocol-only CLI status/events/submit and filesystem-only read-only doctor discovery;
      it does not execute provider binaries, including version probes, in observer mode.
- [x] `gent deps` requires explicit consent, then invokes only fixed, shell-free vendor installer
      commands for public Claude/Codex dependencies and waits for their terminal result; it never
      installs private Claurst components or runs during `gent doctor`.
- [x] Capability-gated local event attachment: initial replay, snapshot resync, cursor-ordered
      bounded batches, client acknowledgements, and `gent events --follow` over the existing IPC.
      The daemon polls its durable ledger at a bounded interval; it does not yet claim a producer
      notification path or automatic client reconnect.
- [x] Credential-free private external-provider bridge DTOs and dedicated handshake/lifecycle
      wire frames; no private bridge endpoint or implementation is composed by `gentd`.
- [x] End-to-end local-IPC smoke tests for Unix sockets and Windows named pipes,
      including daemon, client, receipt, decision, and event ordering.
- [x] Phase-0 coverage-manifest structural validator and CI checks.
- [x] Phase-0 public-driver capture inventory with an explicit provenance-aware
      live-evidence gate.
- [x] A bounded, redaction-first Codex app-server JSON-RPC capture harness for
      documented approval, plan, compaction, MCP, interrupt, and steering
      scenarios, with deterministic fake transport tests. It is a capture tool,
      not evidence: no newly captured Codex matrix cell is claimed until a
      reviewed live fixture is explicitly admitted to the manifest.
- [x] Fail-closed cross-linking from passed public-provider coverage records to
      recorded driver transcripts with matching provider/version/platform/transport.
- [x] Digest-bound signed compatibility entries, trusted-key revocation, fixed-expiry offline cache,
      immutable durable run-version locks, and a daemon-owned resolver component. The shipped
      observer still denies public lifecycle requests before resolving an executable; enabling
      authority additionally requires composition approval and real-provider evidence.
- [x] When a fresh authorized lock is available, a changed executable produces a separately
      reserved child run with immutable lineage; it never mutates the parent run or silently
      substitutes a provider.
- [x] An already-created durable chat run can atomically acquire its first immutable executable
      lock and daemon lease. Exact retries are stable; competing same-epoch owners and any lock
      replacement are rejected before a provider spawn is possible.
- [x] Immutable, restart-safe provider-native session bindings; resume ignores the legacy client wire value.
- [x] Lease- and session-bound durable run lifecycle projections, with cursor-monotonic restart recovery.
- [x] Durable immutable conversation → run → turn identity, provider-switch lineage, and monotonic turn lifecycle transitions.
- [x] Durable workspace → repository → worktree identities, deliberately separate from lease arbitration and Git execution.
- [x] Durable worktree-scoped Git-operation records with optimistic, terminal-safe lifecycle transitions;
      no Git process execution is enabled.
- [x] Durable workspace tool-source declarations for MCP, built-in, and host integrations;
      declarations contain no credentials/endpoints and cannot connect or spawn a source.
- [x] Durable ordered run checkpoints with monotonic event cursors and digest-only state references;
      checkpoint persistence cannot resume or leave a provider process running.
- [x] Negotiated `gent permissions show|set` stores append-only, secret-free revisions: Default,
      Plan, Auto-Accept Edits, Autonomous, or persistent Bypass after one explicit confirmation.
      The pure evaluator keeps Plan non-escalating; broad modes fail closed without containment and no policy starts a provider.
- [x] Read-only conversation status derivation from durable lineage and run projections, with no provider session disclosure.
- [x] Durable title/recap provenance records: source turns, provider/model version, input digest,
      immutable lineage, and atomic supersession.
- [x] Capability-gated, same-socket `gent conversation status` transport; it creates no receipt and does not use command or event frames.
- [x] Capability-gated `gent conversation timeline` transport for ordered run/turn lineage and
      title/recap provenance metadata; it excludes artifact text and provider-native sessions.
- [x] Capability-gated `gent conversation list` transport for reverse-created durable identities
      and run counts; it is content-free discovery for a future terminal conversation browser.
- [x] `gent` / `gent --conversations` read-only terminal shell with content-free local discovery,
      selection, disabled composer, and unavailable model/effort/mode controls in observer mode.
- [x] Unix-only `gent conversation content` reads of user-authored prompts, with conversation-bound
      keyset cursors, a SQLite page budget, and an exact protocol-frame budget; no provider output
      or observer-mode write path is exposed.
- [x] Explicit `gentd --agent-chat-authority` local profile: negotiated `gent chat create`,
      `send`, and `queue` persist through receipt and epoch fences without composing any provider,
      MCP, Git, or private-bridge effect. Accepted prompts expose durable `awaitingProvider` or
      `queued` delivery rather than claiming execution. The default daemon remains observer-only.
- [x] Receipt-backed `gent chat switch` creates a new immutable, selected child run only when
      the expected parent is still the durable current run. It records a frozen conversation
      history ordinal before the child begins; retries are stable and later prompts target the
      child. No provider, MCP, Git, or private bridge is launched or inspected by this switch.
- [x] Unix local-host privacy boundary: a non-symlink, owner-only daemon data directory and an
      owner-only Unix socket constrained beneath it before the ledger is opened.
- [x] Additive normalized lifecycle signals for root phase and explicit generation activity,
      subagent and command work, and attention; lease-owned durable projections preserve them.
      Waiting work is derived from activity rather than inferred from root phase.
- [x] Versioned, content-free `ConversationActivity` DTOs and a pure conversation-scoped reducer
      with epoch/cursor fencing, terminal dominance, decision priority, descendant liveness, and
      stale-turn rejection. Complete reducer checkpoints are journaled and cursor-resumable per
      conversation/run. An authority-gated runtime service fences facts before reduction and
      persists exact reducer checkpoints; it remains intentionally unadvertised and uncomposed by
      the observer daemon.
- [x] Dedicated `conversation-activity-v1` snapshot/delta protocol frames bind reads to a
      conversation, run, and durable cursor, with a bounded-delta snapshot fallback. A dedicated
      daemon adapter and protocol-only `gent conversation activity` reader exist, but the observer
      daemon does not advertise or serve the capability because it has no authoritative provider
      fact ingress. The reader rejects mixed-run, regressing, or internally inconsistent deltas.
- [x] Content-free runtime-release metadata and a pure update eligibility/lifecycle reducer. It
      preserves a closed-ingress boundary for health, activation, and failure, and refuses rollback
      after a forward-only schema release. Runtime-owned Ed25519 trust, signer revocation, manifest
      shape validation, and an atomically stored offline cache revalidate every read. Durable attempt
      checkpoints and fakeable source/staging/health/bootstrapper ports exist. An uncomposed,
      authority-gated planner makes observer mode a no-op and closes ingress before persisting an
      incompatible release as read-only. Its uncomposed executor persists staging, health, and
      bootstrapper-handoff transitions, refuses to replay incomplete effects after restart, and
      keeps ingress closed after health or activation begins; no live release source, platform
      staging adapter, binary swap, or observer update API exists yet. An explicit
      `--runtime-update-check-authority` composition can advertise the report-only
      `runtime-update-check-v1` contract after loading a trusted cached signed release;
      it revalidates the signature and expiry on every request and has no fetch,
      durable-write, archive-download, staging, or activation capability. `gent update check`
      now requires that negotiated daemon capability rather than performing client-owned discovery.
- [x] Signed, expiring release-index DTOs and runtime trust verification for target-specific,
      tag/version-consistent, digest-bound release-manifest offers. A signed external helper
      provides default LaunchAgent/systemd-user/Scheduled Task scheduling, serialized idle-only
      checks, bounded retry backoff, and tag-bound bootstrap verification before it delegates
      activation. It is not a daemon scheduler or observer update authority.
- [x] Authority-gated, versioned `runtime-maintenance-v1` status reads expose one durable
      update attempt's exact stage/failure/revision and host ingress state through negotiated
      local IPC. It is unavailable in observer mode and cannot fetch, schedule, stage, or
      activate an update; `gent update status --attempt-id <id>` is its protocol-only client.
- [x] Pure normalized driver frames and declarative adapter interpreter.
- [x] Pure documented Claude stream-JSON and Codex app-server handshake/normalizers with
      ordered synthetic transcript replay; these preserve only typed facts and do not
      constitute real-provider evidence or activate a process.
- [x] Content-safe normalized tool-activity facts and an immutable, exact-match Gent tool
      taxonomy; provider payloads cannot choose a presentation category or retain tool I/O.
- [x] Fixture-tested driver session recovery, output bounds, interrupt policy, process fakes,
      locked public-process launching, and minimal Claude stream-JSON/Codex app-server launch specifications.
- [x] Pure, session-bound NDJSON command encoding for documented Claude stream-JSON and Codex
      app-server user-message frames; it validates only and does not write to a provider process.
- [x] Locked process ownership includes explicit standard-input frame delivery, tested against a
      local process; observer-mode `gentd` does not invoke this driver edge.
- [x] `gentd` composes the public-run service only in hard-coded observer mode; every lifecycle
      request is denied before executable inspection, lock capture, or process launch.
- [x] A dormant `PublicDriversRuntime` composition seam accepts only a validated approval whose
      evidence reference and pinned SHA-256 match the exact signed compatibility envelope. Its
      injected runner/resolver path connects run, lifecycle, and activity ingress, including
      activation of an existing same-provider chat run; no startup flag reaches it, so the shipped
      daemon remains hard observer and cannot launch a provider through this seam.
- [x] Bounded stdout output pump connects chunk-tolerant NDJSON framing to the existing
      supervisor frame buffer and pure session reducer, retaining FIFO frames across backpressure
      without retaining oversized reads or lines.
- [x] Production process stdout is delivered through a bounded queue into that pump; direct
      process waits drain the queue safely, and runner-owned monotonic deadlines execute the
      interrupt → terminate → kill ladder or cancel it on process exit.
- [x] Pure Git porcelain parsing plus a bounded, fixed-argv, canonical-path Git-status
      executor behind a receipt-backed, worktree-lease-fenced authority service. It returns
      only a count and digest. A dormant injected-executor seam accepts an explicit approved
      status token; observer mode denies it before a receipt, lease, or process is created, and
      every mutation remains rejected and uncomposed.
- [x] Receipt-backed, source-lease-fenced MCP connector lifecycle coordination. It resolves
      only durable credential-free MCP declarations, persists requested → connecting → terminal
      state, and never replays an accepted receipt after restart. A dormant injected-executor
      seam requires an approved evidence/registry digest and validates declarations inside the
      receipt/lease flow; normal startup remains observer-only and starts no MCP process/network.
- [x] Receipt-backed user conversation-prompt persistence. Prompt text is stored only in a
      dedicated SQLite message ledger while receipt/event payloads retain only identities,
      digest, byte length, and assigned sequence. Recovery safely retries its one idempotent
      database transaction; it is uncomposed by observer-mode `gentd` and never starts a provider.
- [x] Worktree lease policy and MCP registry/lifecycle.
- [x] Fail-closed evidence-record validation, including expired temporary-exception rejection.
- [x] macOS/Linux/Windows CI matrix for supported local-host transport targets.
- [x] Pinned public-library API compatibility gate against the `main` baseline.
- [x] Enforced 90% workspace line-coverage gate.
- [x] Deterministic release packaging, checksum/manifest verification, portable Ed25519
      runtime-metadata signing, and tag-only GitHub OIDC keyless-signing workflow for `gent` and
      `gentd` artifacts.
- [x] Signed macOS/Linux and Windows x86_64 release bootstraps stage `gent` and
      `gentd` as an immutable pair before atomically selecting them. The installer
      serializes the full transaction, byte-compares any existing release to the
      verified archive, publishes launchers before selection, and fsyncs Unix
      pointer transitions. Windows uses a validated `current.json` file plus a
      signed native launcher rather than a `.cmd` forwarding wrapper; offline
      tests cover first install, tamper refusal, forced update, live-host refusal,
      and manifest-tamper preservation of the previous pair.
- [x] When a release publishes a Sigstore-verified public runtime trust document and
      target release metadata, the staged `gentd` independently validates its signature,
      expiry, exact archive digest/size/name, target, and version before atomically writing
      a revalidatable local cache. The installer copies that trust/cache pair only after this
      one-shot verification; local end-to-end tests use packaged real Gent binaries.
- [x] User-invoked `gent update apply` verifies a tag-bound Sigstore installer
      bootstrap before external handoff, requires a target archive digest and
      explicit consent, and passes the selected data directory to activation.
      An installed Unix update requires the signed supervisor: it stages the
      exact release, health-checks local IPC, waits for the old host lock,
      atomically selects the pair, and rolls back after successor-health failure.
      A missing supervisor rejects the handoff without changing the active pair.
      It never performs in-process replacement or background release polling.
- [x] `gent update auto enable|status|disable|run` delegates only to the signed
      installed external helper. GitHub `latest` is an untrusted stable-tag hint;
      the helper repeats tag-bound Sigstore bootstrap verification and invokes
      the same paired, idle-lock, staged-health, rollback-aware installer path.
- [x] Read-only stable-channel update discovery: untrusted GitHub metadata only
      locates a tag; the matching target manifest and Sigstore bundle must verify
      before a candidate/digest is shown. Missing network or `cosign` truthfully
      yields `releaseMetadataUnavailable` and cannot authorize an update.
- [x] Standalone discovery-first onboarding documentation with explicit dependency consent.
- [x] Read-only `gent onboarding` projection with exactly Gent/Claurst, Claude, and Codex branches;
      it derives readiness only from `gent doctor`, never starts a provider or performs auth/install/download work.
- [x] The daemon's target product boundary is recorded: it will own agent-chat conversations,
      sessions, prompts, Claude/Codex drivers, the private Claurst bridge port, MCP, and Git;
      a future Flutter caller invokes `gent` rather than a provider executable. Device pairing
      and application-specific UI automations remain Flutter-owned. A future agent-chat
      `gent-automations` domain is separate and has no current `gentd` protocol or executor.
- [x] Public capability-gated future agent-chat contract values: provider-neutral conversation
      summary/detail and normalized transcript pages, model/effort/mode selection, and typed
      create/send/queue/interrupt/decision/cursor-subscription frames with request and receipt
      correlation. They are deliberately uncomposed by observer-mode `gentd`; no frame activates
      a provider or creates a write surface today.
- [x] Dedicated uncomposed agent-chat conversation and prompt boundaries. Their approved-runtime
      paths atomically persist a conversation/root run/selection or a receipt/turn/protected user
      message/ordinal respectively; retries are stable and observer authority returns before any
      ledger write. They do not start providers or advertise an IPC capability.
- [x] Language-neutral local IPC fixtures with canonical JSON and exact length-prefixed wire
      bytes for negotiation, errors, cursor replay, and every reserved agent-chat frame. The
      validator rejects a composed declaration for any reserved capability, so fixture presence
      cannot be mistaken for observer-mode authority.

## Required before Gent is a live app backend

1. [ ] `gentd` remains observer/intent-only: it must not route live Claude, Codex, Claurst,
   MCP, or Git work until each authority gate is proven. The dormant seams are not a claim of
   live authority.
2. [ ] There is no authoritative provider-lifecycle ingress yet. The required realtime
   browse/create/prompt/follow-up/reconnect path is in [the client contract](realtime-agent-chat-client-plan.md);
   until approved, Flutter and terminal clients must not treat activity as live truth.
3. [ ] The strict public evidence program has six Claude/Codex cells. Two Codex cells are
   recorded; four remain: Claude persistent-permission, compaction, malformed-tolerance, and
   Codex malformed-tolerance. Captures must be redacted, scenario-specific, and live. A malformed
   capture additionally needs a documented provider-emitted fault control, diagnostic, and
   following ordinary frame; proxy or injected corruption is rejected.
   Installed Claude Code 2.1.233 has neither `--permission-prompt-tool` nor a structural bounded
   compaction signal (only `--permission-mode` and `--autocompact` at 100k–1M tokens); Codex CLI
   0.144.1 and isolated 0.147.0 inspection expose no provider-output fault control. No safe capture
   is available until that changes.
4. [ ] The authenticated Claurst bridge and its CI evidence belong only in app-owned private
   code. Public Gent must never contain Claurst credentials, endpoints, or routing implementation.
5. [ ] No legacy migration or deployed fence-aware legacy release is required: this is a
   zero-user, single-developer cutover. A future Flutter launch must nevertheless enforce protocol
   compatibility and exactly one active writer/host epoch.
6. [x] Production release automation has its dedicated GitHub Actions signing secret and matching
   public key/id configuration (`runtime-2026-08`). `v0.1.14` is published with all 46 assets,
   Sigstore sidecars (including the versioned runtime-release index), and successful hosted plus
   independent clean-install, terminal-IPC, automatic-update-status, and supervisor-rejection checks.
7. [ ] Reviewed-plan storage, exact approval/rejection, and receipt-backed context-boundary child reservations exist but remain unadvertised until lifecycle/evidence authority is approved; see [reviewed-plan execution](agent-chat-execution-plan.md). `gent-canvas`, `gent-forge`, live MCP/Git authority, and seamless live provider switching are also follow-on work.
8. [ ] Provider-auth discovery and consented Claude/Codex login require the typed `askTool`
   contract, sandboxed authority, locked binaries, and redacted live evidence; see [provider-auth-plan.md](provider-auth-plan.md).

Also required: a separately authorized Flutter integration must use the negotiated, long-lived
`gentd` connection for agent-chat work and must not launch provider binaries directly. Pairing and
application-specific UI automations stay Flutter-owned. The client boundary is
`docs/flutter-handoff-v1.md`.

## Recorded follow-on scope

The next product-scope request is recorded here, not implied by the current
observer milestone. After the evidence and authority gates above, Gent will add
native `gent-canvas`, `gent-forge`, and `gent-automations` domains. They will be
separate modules with typed ports and reducers, and `gentd` will be their only
composition root. Device pairing and the Flutter application's non-agent UI
automation remain app-owned.

Provider selection must remain a durable child-run transition. Switching among
Claude, Codex, or the private Claurst bridge must preserve the provider-neutral
conversation history and lineage without rewriting an existing run or exposing
Claurst credentials. The later native domains and this continuity contract require
their own protocol, persistence, receipts, observer-disablement, and live-evidence
work before they are advertised.

The coverage manifest blocks authority transfer while real evidence is absent;
recorded provider evidence is external, never a placeholder. A future app launch enforces one-writer/host-epoch.
