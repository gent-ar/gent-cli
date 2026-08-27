# App driver cutover inventory

The direct app Claude, Codex, and Claurst adapters are protocol references for
Gentd. They are not a fallback path. Gentd now owns the shared provider-neutral
conversation contract and the native app uses it over local IPC.

| App behavior | Gentd owner | Native client behavior |
| --- | --- | --- |
| Conversations and runs | Durable conversation/run lineage | Uses daemon IDs and immutable switches |
| Prompt and follow-up | Receipt-backed intent and lifecycle | Sends typed prompt intent and follows exact turn |
| Streaming, tools, thinking, subagents | Normalized transcript and activity facts | Renders durable Gentd facts |
| Permissions | Typed binding and settlement | Reads and responds through Gentd |
| MCP | Daemon provider configuration | Generates one shared config file |
| Models | Claude/Codex selection or curated local Claurst catalogue | Selects through immutable run settings and renders local downloads |
| Attachments | Daemon staging and provider-neutral projection | Stages bytes and submits durable IDs only |

## Direct-adapter semantics already mapped

The app's Claude driver encoded images inline and text files as local references.
The Codex driver encoded local-image paths and file references. Those path-based
forms cannot cross the Gentd boundary. Gentd staging replaces them with opaque
attachment IDs, metadata, and daemon-owned bytes. A runner must project those
bytes only after it resolves the durable turn attachment links.

Claurst is a dependency-backed local provider. It receives no hosted endpoint,
credential, or app routing fallback. Gentd selects a curated downloaded model
and its local llama.cpp runtime.

## Open integration items

- Complete provider-neutral attachment projection in the daemon runners.
- Complete exact-run interrupt routing in standalone lifecycle authority.
- Finish the remaining live-evidence matrix and platform coverage.
- Delete equivalent direct app execution only after the matching Gentd behavior
  is verified with no fallback route.
