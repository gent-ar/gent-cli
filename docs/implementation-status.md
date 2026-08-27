# Implementation status

This document describes the current standalone Gent integration, not the old
observer-only milestone. `gentd` remains the sole ledger writer and provider
owner. The terminal and native app are clients of its negotiated local IPC.

## Working standalone authority

Start `gentd` with `--standalone-authority`. It composes one durable agent-chat
runtime and advertises agent-chat intents, conversation reads, transcripts,
permissions, turn follow, activity reads, attachment staging, and curated local
model operations.

- Claude and Codex are daemon-owned processes. The standalone profile accepts
  exact executable paths and can provision its managed standalone dependencies.
- Claurst is a Gent provider, not a fork. It uses Claurst as a dependency with
  a curated ungated local-model catalogue and llama.cpp `llama-server` runtime.
  Model downloads are owned by Gentd and report durable progress.
- Claude, Codex, and Claurst share conversation, run, turn, transcript,
  permission, model/provider-selection, MCP configuration, and cursor contracts. A provider
  switch creates a child run with Gent-owned history; it never transfers a
  provider-native session across providers. A switch into a fresh Gentd run
  carries the exact prior transcript in the first prompt's provider-neutral
  history envelope.
- The native app launches the packaged `gentd` pair, passes its generated MCP
  configuration, negotiates IPC, and never launches a provider directly.
- Native attachments are staged as daemon-owned bytes and prompts carry only
  durable attachment IDs. Original app filesystem paths never enter the prompt
  protocol.

The hard-observer profile still exists for read-only and test use. It is not
the standalone product profile and must not be used to judge standalone
capability availability.

## Verified boundaries

- Length-prefixed versioned IPC, capability negotiation, host epochs, receipts,
  idempotency, SQLite recovery, cursor pages, immutable conversation/run/turn
  lineage, and normalized transcript/activity facts.
- Claude/Codex stream normalization, permissions, tool activity, model/effort/mode
  selection, process ownership, and MCP config injection are owned by Gentd.
- Claude, Codex, and Claurst active-run interruption is received through typed IPC and
  routed to their retained lifecycle owner. The terminal state remains provider-derived.
- Claurst local-model catalogue, download progress, local runtime preparation, and
  normalized ACP lifecycle are owned by Gentd.
- App conversations use Gentd conversation IDs and current run IDs. The app
  renders daemon transcript pages and follows exact daemon turns.

## Known capability limits

- Claude, Codex, and Claurst resolve verified daemon-owned attachment bytes at provider launch.
  Claurst projects image attachments only when the negotiated ACP image capability is present;
  unsupported media fails before ACP launch.
- Codex child-run correlation and bounded requestUserInput/MCP elicitation answer relay are durable. Claude background-agent launch and terminal lifecycle correlation is durable; sidecar thinking, text, and tool-result activity is
  not yet exposed and must not be synthesized. Claurst v0.1.7's ACP surface exposes no child
  session or subagent capability, so addressable subagent messaging is rejected rather than
  represented as if it were supported. Claurst's internal coordinator remains available only
  inside its own TUI/runtime surface. See the [Claurst ACP contract](https://github.com/Kuberwastaken/claurst#editor-integration-agent-client-protocol).
- The strict live-evidence matrix still lacks Claude compaction, Claude
  malformed-tolerance, and Codex malformed-tolerance recordings. Do not invent
  recordings.
- The native app currently drops `requestId` while projecting global local-model
  download events and filters only by model. Gentd already emits the durable
  prompt-bound request identity; the native projection must retain and filter it
  before automatic downloads are release-ready.
- Claurst requires the packaged Claurst and llama.cpp runtime plus a curated local model. A first prompt for
  an absent model automatically downloads it with correlated durable progress; it never routes to a hosted endpoint.
  The curated Qwen3 8B Q4_K_M model is pinned in `models.json`, verified by digest and Hugging
  Face revision. Standalone ACP transport,
  permission relay, MCP conversion, settings embedding, lifecycle tests, and daemon-owned download
  progress pass. A clean macOS standalone run downloaded the curated 3B model,
  verified it, and completed a streamed Claurst ACP turn. Windows and Linux
  release-hardware turns remain required before making cross-platform performance claims.

## Required release checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
python3 tools/check-architecture.py
git diff --check
```

The native app must use only negotiated Gentd capabilities. A missing
capability is unavailable; it never authorizes a direct Claude, Codex, or
Claurst route.

The workspace test and architecture gates pass. Workspace clippy remains a separate cleanup item:
the current lint profile rejects existing public `Result` traits without `# Errors` documentation;
adding lint suppressions would violate the repository's zero-comment rule.
