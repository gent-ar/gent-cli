# Implementation Status

This document records implemented repository work separately from migration
evidence. A checked box means code and deterministic tests exist here; it does
not claim provider or app compatibility evidence that has not been recorded.

## Implemented foundations

- [x] Fifteen-crate Rust workspace with enforced dependency law.
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
- [x] Fail-closed cross-linking from passed public-provider coverage records to
      recorded driver transcripts with matching provider/version/platform/transport.
- [x] Digest-bound signed compatibility entries, trusted-key revocation, fixed-expiry offline cache,
      immutable durable run-version locks, and a daemon-owned resolver component. The shipped
      observer still denies public lifecycle requests before resolving an executable; enabling
      authority additionally requires composition approval and real-provider evidence.
- [x] When a fresh authorized lock is available, a changed executable produces a separately
      reserved child run with immutable lineage; it never mutates the parent run or silently
      substitutes a provider.
- [x] Immutable, restart-safe provider-native session bindings; resume ignores the legacy client wire value.
- [x] Lease- and session-bound durable run lifecycle projections, with cursor-monotonic restart recovery.
- [x] Durable immutable conversation → run → turn identity, provider-switch lineage, and monotonic turn lifecycle transitions.
- [x] Durable workspace → repository → worktree identities, deliberately separate from lease arbitration and Git execution.
- [x] Durable worktree-scoped Git-operation records with optimistic, terminal-safe lifecycle transitions;
      no Git process execution is enabled.
- [x] Durable workspace tool-source declarations for MCP, built-in, and host integrations;
      declarations contain no credentials/endpoints and cannot connect or spawn a source.
- [x] Durable workspace automation-execution records with duplicate-trigger prevention and
      terminal-safe transitions; scheduler, webhook, and process execution remain disabled.
- [x] Durable ordered run checkpoints with monotonic event cursors and digest-only state references;
      checkpoint persistence cannot resume or leave a provider process running.
- [x] Durable, append-only workspace provider-permission policy revisions with canonical secret-free allow-lists.
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
      conversation, run, and durable cursor. The observer daemon does not advertise or serve the
      capability because it has no authoritative provider fact ingress.
- [x] Content-free runtime-release metadata and a pure update eligibility/lifecycle reducer. It
      preserves a closed-ingress boundary for health, activation, and failure, and refuses rollback
      after a forward-only schema release. Runtime-owned Ed25519 trust, signer revocation, manifest
      shape validation, and an atomically stored offline cache revalidate every read. Durable attempt
      checkpoints and fakeable source/staging/health/bootstrapper ports exist. An uncomposed,
      authority-gated planner makes observer mode a no-op and closes ingress before persisting an
      incompatible release as read-only. Its uncomposed executor persists staging, health, and
      bootstrapper-handoff transitions, refuses to replay incomplete effects after restart, and
      keeps ingress closed after health or activation begins; no live release source, platform
      staging adapter, binary swap, or observer update API exists yet.
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
- [x] Bounded stdout output pump connects chunk-tolerant NDJSON framing to the existing
      supervisor frame buffer and pure session reducer, retaining FIFO frames across backpressure
      without retaining oversized reads or lines.
- [x] Production process stdout is delivered through a bounded queue into that pump; direct
      process waits drain the queue safely, and runner-owned monotonic deadlines execute the
      interrupt → terminate → kill ladder or cancel it on process exit.
- [x] Pure Git porcelain parsing plus a bounded, fixed-argv, canonical-path Git-status
      executor behind a receipt-backed, worktree-lease-fenced authority service. It returns
      only a count and digest, is uncomposed by `gentd`, and observer mode denies it before
      a receipt, lease, or process is created.
- [x] Receipt-backed, source-lease-fenced MCP connector lifecycle coordination. It resolves
      only durable credential-free MCP declarations, persists requested → connecting → terminal
      state, and never replays an accepted receipt after restart. It is uncomposed by `gentd`
      and has no process or network executor implementation; observer mode returns before a
      receipt, lease, connector record, or executor call.
- [x] Receipt-backed user conversation-prompt persistence. Prompt text is stored only in a
      dedicated SQLite message ledger while receipt/event payloads retain only identities,
      digest, byte length, and assigned sequence. Recovery safely retries its one idempotent
      database transaction; it is uncomposed by observer-mode `gentd` and never starts a provider.
- [x] Worktree lease policy, MCP registry/lifecycle, automation policy, and pairing replay semantics.
- [x] Fail-closed evidence-record validation, including expired temporary-exception rejection.
- [x] macOS/Linux/Windows CI matrix for supported local-host transport targets.
- [x] Pinned public-library API compatibility gate against the `main` baseline.
- [x] Enforced 90% workspace line-coverage gate.
- [x] Deterministic release packaging, checksum/manifest verification, and tag-only
      GitHub OIDC keyless-signing workflow for `gent` and `gentd` artifacts.
- [x] Standalone discovery-first onboarding documentation with explicit dependency consent.
- [x] Read-only `gent onboarding` projection with exactly Gent/Claurst, Claude, and Codex branches;
      it derives readiness only from `gent doctor`, never starts a provider or performs auth/install/download work.
- [x] The daemon's target product boundary is recorded: it will own agent-chat conversations,
      sessions, prompts, Claude/Codex drivers, the private Claurst bridge port, MCP, and Git;
      a future Flutter caller invokes `gent` rather than a provider executable. Device pairing
      and application automations remain Flutter-owned and have no `gentd` protocol or executor.

## Intentionally not claimed

- [ ] Real Claude/Codex recordings and installed-provider integration evidence.
- [ ] Authenticated private Claurst bridge evidence (private CI only).
- [ ] MCP hosting, Git mutation/worktree operations, and provider process lifecycle ownership
      in a live daemon. The narrow Git status service above remains dormant until an
      authority-gated host profile is proven.
- [ ] A separately authorized Flutter integration that invokes `gent` for agent-chat work.
      It must not launch provider binaries directly. Device pairing and application automation
      execution stay Flutter-owned and are intentionally excluded from `gentd`.
- [ ] A phase-4 legacy-observer host profile: it must consume a `LegacyEventTap`
      without Rust durable writes, mutation APIs, or worktree leases. The current
      standalone daemon's hard public-provider observer guard does not claim this.
- [ ] Fence-aware legacy app release and authority-transfer state machine.
- [ ] Versioned public `ConversationActivity` projection with durable activity sequence,
      revision/cursor resume, app-compatible fallback, and complete lifecycle race coverage.
- [ ] Signed, staged, health-checked `gentd` self-update with compatibility ranges, safe
      rollback boundaries, and update-under-load recovery evidence.

The coverage manifest blocks an authority-transfer invocation while its real
evidence records are absent. This is deliberate: recorded provider evidence
and a legacy-writer release are external prerequisites, never placeholders.
