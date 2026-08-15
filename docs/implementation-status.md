# Implementation Status

This document records implemented repository work separately from migration
evidence. A checked box means code and deterministic tests exist here; it does
not claim provider or app compatibility evidence that has not been recorded.

## Implemented foundations

- [x] Fifteen-crate Rust workspace with enforced dependency law.
- [x] Protocol negotiation and bounded length-prefixed JSON local IPC.
- [x] SQLite receipt/event ledger, idempotency, event cursors, and epoch checks.
- [x] Durable event snapshots and transactional compaction with explicit stale-cursor resync.
- [x] Durable run and worktree lease arbitration with separate-connection contention tests.
- [x] File-backed SQLite restart recovery for host epoch and cursor-ordered receipt events.
- [x] Pure run-lineage, cursor-deduplicated lifecycle projection, and live-status reducers.
- [x] Pure idempotent decision-settlement reducer with unprovable and recovery-required terminal paths.
- [x] Durable SQLite decision settlement with restart-safe terminal outcomes and optimistic contention handling.
- [x] Protocol-only CLI status/events/submit and read-only doctor discovery.
- [x] End-to-end local-IPC smoke tests for Unix sockets and Windows named pipes,
      including daemon, client, receipt, decision, and event ordering.
- [x] Phase-0 coverage-manifest structural validator and CI checks.
- [x] Signed compatibility entries, trusted-key revocation, fixed-expiry offline cache, and immutable durable run-version locks.
- [x] Pure normalized driver frames and declarative adapter interpreter.
- [x] Fixture-tested driver session recovery, output bounds, interrupt policy, process fakes, and locked public-process launching.
- [x] Pure Git porcelain parsing, worktree lease policy, MCP registry/lifecycle, automation policy, and pairing replay semantics.
- [x] Fail-closed evidence-record validation, including expired temporary-exception rejection.
- [x] macOS/Linux/Windows CI matrix for supported local-host transport targets.

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
