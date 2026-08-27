# Gent CLI to Native App integration plan

## Product rule

Gentd is the source of truth. `gent` and the native app are clients. Native Agent Chat renders Gentd
catalogs, projections and intents; it does not keep a second provider, conversation or permission
implementation.

## Read in this order

1. `gentd-source-of-truth-contract.md` — the non-negotiable ownership and generic-UI rule.
2. `integration-gap-review.md` — the audited blockers, with file-and-line evidence. This is the only
   doc that carries evidence; every other doc defers to it.
3. `implementation-backlog.md` — the ordered work items and their acceptance gates.
4. `native-surface-disposition.md` — every native Agent Chat surface mapped to its final owner.
5. `native-agent-chat-cutover-map.md` — the Flutter cutover architecture and deletion boundary.
6. `onboarding-and-provider-readiness-contract.md` — packaged runtime, prompt admission, install,
   login and cancellation semantics.
7. `agent-chat-parity-inventory.md` and `automation-and-forge-port-contract.md` — surface-by-surface
   detail behind items 4 and 5.
8. `shared-data-dir-and-daemon-ownership.md` — the record of backlog item 1's Rust pass.

Path convention throughout: `crates/…` and `tools/…` are in this repo; `native:…` is
`/Users/ivanmatiasfort/Clouseau/clouseau-app`, relative to its root.

## Gate before native integration starts

Every line below is checkable. Native cutover (backlog item 7) does not begin until all pass.

- `gent data-dir` and the app's resolved data directory print the same string on macOS, Linux and
  Windows, and launching both clients yields exactly one `gentd` process for that directory.
- The terminal renders and mutates every composer control from Gentd catalog records; no permission
  or mode string literal remains under `crates/gent-cli/src/terminal/`.
- `agent-chat-projection-v1` serves the terminal's entire chat screen, and the six capabilities it
  replaces are gone from `crates/gent-runtime/src/catalog.rs`.
- A first prompt to each provider survives model download, CLI install and login without being
  retyped, duplicated or lost.
- Sessions, templates, docs, automations, MCP, Git, fork/resume and checkpoints each have a declared
  capability that the terminal consumes.
- Release CI publishes six archives — macOS x64/ARM64, Linux x64/ARM64, Windows x64/ARM64 — each
  containing `gent`, `gentd`, Node and the Claurst/llama.cpp runtime, and each passing a
  `gent --version` plus `gentd` handshake check on its own architecture.

Only then does Flutter replace its Agent Chat authority with the generic Gentd client. No
compatibility branch and no second durable writer is part of the target.

## Decisions made this pass

All three prior open decisions are resolved — see "Decisions the user made this pass" in
`integration-gap-review.md`:

- Canonical data directory is `<home>/.gentd`. Renamed and built, with a one-time migration from the
  old `<home>/.gent-cli`.
- Daemon shutdown is idle-based, not tied to any one client quitting. Decided and specified; not yet
  built — `implementation-backlog.md` item 1, step 6.
- `/btw` side questions survive, as a new `agent-chat-side-question-v1` capability. Specified; not yet
  built — `implementation-backlog.md` item 5.
