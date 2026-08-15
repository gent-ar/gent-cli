# Implementation Status

This document records implemented repository work separately from migration
evidence. A checked box means code and deterministic tests exist here; it does
not claim provider or app compatibility evidence that has not been recorded.

## Implemented foundations

- [x] Fifteen-crate Rust workspace with enforced dependency law.
- [x] Protocol negotiation and bounded length-prefixed JSON local IPC.
- [x] SQLite receipt/event ledger, idempotency, event cursors, and epoch checks.
- [x] Durable event snapshots and transactional compaction with explicit stale-cursor resync.
- [x] Versioned, checksummed, transactional SQLite migrations with legacy-ledger upgrade tests.
- [x] Durable run and worktree lease arbitration with separate-connection contention tests.
- [x] File-backed SQLite restart recovery for host epoch and cursor-ordered receipt events.
- [x] Pure run-lineage, cursor-deduplicated lifecycle projection, and live-status reducers.
- [x] Pure idempotent decision-settlement reducer with unprovable and recovery-required terminal paths.
- [x] Durable SQLite decision settlement with restart-safe terminal outcomes and optimistic contention handling.
- [x] Protocol-only CLI status/events/submit and read-only doctor discovery.
- [x] End-to-end local-IPC smoke tests for Unix sockets and Windows named pipes,
      including daemon, client, receipt, decision, and event ordering.
- [x] Phase-0 coverage-manifest structural validator and CI checks.
- [x] Phase-0 public-driver capture inventory with an explicit provenance-aware
      live-evidence gate.
- [x] Fail-closed cross-linking from passed public-provider coverage records to
      recorded driver transcripts with matching provider/version/platform/transport.
- [x] Signed compatibility entries, trusted-key revocation, fixed-expiry offline cache, and immutable durable run-version locks.
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
- [x] Additive normalized lifecycle signals for root thinking/waiting/compacting states,
      subagent and command work, and attention; lease-owned durable projections preserve them.
- [x] Pure normalized driver frames and declarative adapter interpreter.
- [x] Fixture-tested driver session recovery, output bounds, interrupt policy, process fakes,
      locked public-process launching, and minimal Claude stream-JSON/Codex app-server launch specifications.
- [x] Pure, session-bound NDJSON command encoding for documented Claude stream-JSON and Codex
      app-server user-message frames; it validates only and does not write to a provider process.
- [x] Locked process ownership includes explicit standard-input frame delivery, tested against a
      local process; observer-mode `gentd` does not expose or invoke this driver edge.
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
