# Implementation Status

This document records implemented repository work separately from migration
evidence. A checked box means code and deterministic tests exist here; it does
not claim provider or app compatibility evidence that has not been recorded.

## Implemented foundations

- [x] Fifteen-crate Rust workspace with enforced dependency law.
- [x] Protocol negotiation and bounded length-prefixed JSON local IPC.
- [x] SQLite receipt/event ledger, idempotency, event cursors, and epoch checks.
- [x] Durable local attachment staging with per-transfer opaque staging keys, fenced progress,
      and receipt-bound follow-up operations,
      retry-safe final content promotion, and content-addressed deduplication; it is not exposed
      through observer-mode `gentd` and does not imply provider attachment support.
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
- [x] Signed compatibility entries, trusted-key revocation, fixed-expiry offline cache, and immutable durable run-version locks. Version-only manifests remain discovery evidence only: authority stays denied until a digest-bound signed schema and daemon-owned resolver can authorize an observed executable.
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
- [x] Additive normalized lifecycle signals for root phase and explicit generation activity,
      subagent and command work, and attention; lease-owned durable projections preserve them.
      Waiting work is derived from activity rather than inferred from root phase.
- [x] Pure normalized driver frames and declarative adapter interpreter.
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
- [x] Pure Git porcelain parsing, worktree lease policy, MCP registry/lifecycle, automation policy, and pairing replay semantics.
- [x] Fail-closed evidence-record validation, including expired temporary-exception rejection.
- [x] macOS/Linux/Windows CI matrix for supported local-host transport targets.
- [x] Pinned public-library API compatibility gate against the `main` baseline.
- [x] Enforced 90% workspace line-coverage gate.
- [x] Deterministic release packaging, checksum/manifest verification, and tag-only
      GitHub OIDC keyless-signing workflow for `gent` and `gentd` artifacts.
- [x] Standalone discovery-first onboarding documentation with explicit dependency consent.

## Intentionally not claimed

- [ ] Real Claude/Codex recordings and installed-provider integration evidence.
- [ ] Authenticated private Claurst bridge evidence (private CI only).
- [ ] MCP hosting, Git execution/worktree operations, automation execution, pairing
      transport, and provider process lifecycle ownership in a live daemon.
- [ ] Observer-mode comparison with the legacy host.
- [ ] Fence-aware legacy app release and authority-transfer state machine.

The coverage manifest blocks an authority-transfer invocation while its real
evidence records are absent. This is deliberate: recorded provider evidence
and a legacy-writer release are external prerequisites, never placeholders.
