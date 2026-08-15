# Implementation Status

This document records implemented repository work separately from migration
evidence. A checked box means code and deterministic tests exist here; it does
not claim provider or app compatibility evidence that has not been recorded.

## Implemented foundations

- [x] Fifteen-crate Rust workspace with enforced dependency law.
- [x] Protocol negotiation and bounded length-prefixed JSON local IPC.
- [x] SQLite receipt/event ledger, idempotency, event cursors, and epoch checks.
- [x] Pure run-lineage and live-status reducers.
- [x] Protocol-only CLI status/events/submit and read-only doctor discovery.
- [x] Phase-0 coverage-manifest structural validator and CI checks.
- [x] Signed compatibility-entry verification and executable run-version locks.
- [x] Pure normalized driver frames and declarative adapter interpreter.
- [x] Fixture-tested driver session recovery, output bounds, interrupt policy, and process fakes.
- [x] Pure Git porcelain parsing, worktree lease policy, MCP registry/lifecycle, automation policy, and pairing replay semantics.
- [x] Fail-closed evidence-record validation for authority-transfer invocations.

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
