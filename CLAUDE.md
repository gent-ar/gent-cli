# Gent CLI

Read `docs/continuation-handoff.md` first, every session — it is the authoritative record of
what's implemented, what's next, and why. It supersedes any stale assumption in this file.

## The native app is the reference implementation for provider protocol behavior

`/Users/ivanmatiasfort/Clouseau/clouseau-app` already has working, production, currently-shipping
drivers for Claude Code and Codex:

- `app/lib/model/agent_adapters/driver/claude_driver.dart` — Claude Code's `stream-json`
  stdin/stdout protocol: spawn args, `control_request`/`control_response` permission relay
  (including persistent/session-scoped grants via `updatedPermissions`), tool-use parsing,
  sub-agent transcript correlation, interrupt handling, plan-mode encoding.
- `app/lib/model/agent_adapters/driver/codex_driver.dart` (+ its `codex/*.dart` parts) — Codex's
  `app-server` JSON-RPC protocol: thread/turn lifecycle, tool/item events, sub-agent (collab)
  thread correlation, approval requests, compaction (`thread/compacted`), steering, interrupt.
- `app/lib/model/agent_adapters/generic_adapter.dart` + `app/assets/data/adapter_registry_seed.json`
  — the declarative manifest driving real CLI spawn argv (exact flags, exact order, exact `when`
  conditions) and the classification/normalization rules for both vendors.
- `app/lib/model/agent_adapter.dart` — the shared normalized event vocabulary
  (`AgentStreamEvent` subtypes: turn start/end, tool start/output/done, thinking/activity deltas,
  child start/terminal, compaction, auth/stream errors) both drivers reduce into. This is the
  proven provider-neutral mapping gent-cli's own `NormalizedProviderEvent`/protocol types should
  match.

**Before designing, implementing, or debugging anything involving how Claude, Codex, or their
CLIs actually behave — spawn flags, permission/control protocol, plan mode, sub-agents,
compaction, tool events, error classification — read the relevant file(s) above first.** That
code is battle-tested against real, current CLI versions in a shipping product. Do not guess CLI
behavior, do not design an evidence-capture scenario from first principles, and do not assume a
capability is missing (e.g. an undocumented flag) without checking whether the app already uses
it successfully. Concrete precedent: gent-cli's own docs incorrectly claimed Claude Code lacked
`--permission-prompt-tool`, based on the flag being absent from `--help` output — checking
`claude_driver.dart` (which uses it unconditionally, in production) and then verifying live
(`claude --permission-prompt-tool stdio --version` exits 0) immediately disproved it. See
`docs/continuation-handoff.md`'s "Evidence status" section and the `c340500` commit.

This is a reference-and-verify relationship, not a dependency. gent-cli:
- Never embeds, imports, or shells out to the Flutter app's code at runtime.
- Still independently verifies every behavior it composes (its own evidence-capture,
  compatibility manifests, authority preflight) — the app's code informs the design and saves
  rediscovery time; it is not itself gent-cli's evidence or authority source.
- Ports proven *protocol knowledge* (what flag does what, what frame shape means what, how a
  persistent permission is represented, how compaction is signaled) into gent-cli's own Rust
  types and reducers — never ports Dart code or architecture wholesale.

## Non-negotiable architecture

See `docs/continuation-handoff.md`'s "Non-negotiable architecture" section — `gentd` is the only
composition root and ledger writer; `gent` stays protocol-only; `gent-core` stays pure; public
Gent never contains Claurst credentials/endpoints/routing; default `gentd` is hard observer.
Every hand-authored source/config/document/script is at most 300 lines
(`python3 tools/check-architecture.py`). Commits/pushes require the user's explicit approval.
Never touch `/Users/ivanmatiasfort/Clouseau/clouseau-app` during gent-cli work.
