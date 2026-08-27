# Realtime agent-chat client contract

Gentd standalone authority is the realtime backend for both `gent` and the
native app. One profile has one Gentd writer, one durable ledger, and one
multiplexed IPC endpoint. Clients negotiate capabilities, issue typed commands,
read bounded durable pages, and follow exact turns.

## Required client flow

1. Start or locate `gentd --standalone-authority`.
2. Negotiate the capabilities the client implements.
3. Create or open a durable conversation and retain its current run ID.
4. Send typed prompts, then follow the accepted exact conversation/run/turn.
5. Read transcript, activity, and pending permission state from Gentd.
6. On reconnect, epoch change, or resync, reload pages and continue from the
   durable cursor. Do not reconstruct state from provider output or app memory.

Snapshots, recovery caches, mirrored state, and state replacement are prohibited.
An in-memory view is optional and disposable, never serialized or sent as authoritative state.

Provider/model/effort/mode changes are immutable child runs. Context is Gentd
history, not a provider-native session transfer. Claude, Codex, and Claurst all
use this same client contract; Claurst runs local curated models through
llama.cpp.

## Shared boundaries

- Gentd owns provider processes, normalized transcript/activity facts,
  permissions, MCP configuration, session bindings, and process shutdown.
- Clients own presentation, local input picking, and reconnect UI only.
- Files are staged into daemon-owned storage before prompt submission. Prompt
  input contains attachment IDs, never source paths.
- A missing or rejected capability is an authoritative Gentd result. It never
  permits a direct provider route.

## Current gaps

Provider-neutral attachment projection and exact active-run interrupt routing
remain incomplete in the standalone lifecycle. Clients are wired to the typed
contracts but must present their daemon result until those paths settle.
