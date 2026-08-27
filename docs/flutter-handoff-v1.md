# Flutter Gentd handoff

Flutter is a Gentd IPC client. It starts or locates the installed Gent pair,
connects to the private data-directory endpoint, negotiates capabilities, and
renders Gent-owned durable state. It never starts Claude, Codex, Claurst,
llama.cpp, MCP, Git, or a second ledger writer.

## Connection

Start the packaged daemon with `--standalone-authority` and the app-generated
MCP configuration path. On macOS and Linux the endpoint is
`<data-dir>/gentd.sock`; Windows uses the data-directory-derived named pipe.
The wire format is UTF-8 JSON framed by a four-byte big-endian length. Protocol
version `1` is the current range.

For every connection, send `hello`, require `negotiated`, and use only the
returned capability intersection. On a disconnect, epoch change, expired
cursor, or `resyncRequired`, reconnect, negotiate, reload bounded durable
pages, then continue from the acknowledged cursor.

An in-memory view is disposable and non-authoritative. If a cursor is not accepted, reload from ordinal/cursor zero and replay immutable facts.

## Standalone capability surface

| Need | Capability |
| --- | --- |
| Conversations, selection, prompts, queue, interrupt | `agent-chat-intents-v1` |
| Conversation list and detail | `agent-chat-conversations-v1` |
| Normalized transcript pages | `agent-chat-transcript-v1` |
| Exact live-turn stream | `agent-chat-turn-follow-v1` |
| Permissions | `agent-chat-permissions-v1` |
| Tool and subagent activity | `conversation-activity-v1` |
| Curated Claurst models and downloads | `local-models-v1` |
| Daemon-owned file staging | `attachments-v1` |

The Flutter app carries the durable Gentd conversation and run identities. A
provider/model/effort/mode switch is a typed immutable child-run operation.
Provider-native IDs are never app state and never cross a provider boundary.

## Attachments and local models

The app stages local bytes through `attachments-v1`. Staging sends metadata,
chunks, and commit operations; a later prompt contains only the resulting
attachment IDs. The daemon owns the blobs and their later provider projection.
The app retains source paths only for its display-local attachment bubble.

Claurst uses Gent's curated ungated local models. The app reads download state,
starts a chosen model download, renders progress, and waits for Gentd to launch
the Claurst plus llama.cpp runtime. It must not use a hosted Claurst fallback.

## Known limits

The native IPC path is wired for attachments and typed exact-run interrupts.
Release readiness still requires native local-model `requestId` projection,
Claude child-activity contract work, and packaged Claurst runs on Windows and
Linux. A clean macOS packaged local Claurst turn is verified. Treat an
unavailable or rejected capability as an explicit Gentd result, never as
permission to use an app driver.
