# Claude and Codex authority port status

The native app's Claude and Codex drivers remain protocol references. Gentd's
standalone authority is the runtime implementation: it owns spawning, stdin,
normalization, permissions, transcript persistence, activity, process shutdown,
MCP configuration, and immutable Gent conversation lineage.

`gentd --standalone-authority` composes Claude and Codex lifecycle hosts. This
is no longer an uncomposed observer-only port plan. The app and terminal use
the resulting normalized IPC surface rather than the native providers.

## Mapped behavior

| Native behavior | Gentd standalone behavior |
| --- | --- |
| Claude stream JSON | Daemon-owned stream normalization and lifecycle facts |
| Codex app-server JSON-RPC | Daemon-owned turn/session lifecycle facts |
| Permissions | Typed daemon binding, decision, and provider response relay |
| Tool and subagent activity | Normalized durable activity facts |
| Model, effort, mode | Immutable Gent run selection |
| MCP | One daemon-loaded app-generated configuration |
| Process interrupt/shutdown | Daemon-owned process-tree lifecycle |

## Remaining accuracy limits

- The strict live-evidence matrix still needs Claude compaction and malformed
  cases plus Codex malformed behavior.
- Claude's native background-agent transcript path is authoritative in the
  provider's `tool_result` receipt (`output_file:` plus `agentId:`). Gentd now
  correlates verified launch receipts and terminal notifications to the parent
  tool, but it still has no daemon-owned sidecar tailer or child-scoped content
  contract. Importing that file would either leak child activity into the root
  transcript or require an invented child identity. The native driver tails the
  receipt path and synthesizes parent-tagged stream frames; content remains
  unavailable until Gent adds the same child-scoped contract.
- Claurst's current ACP surface does not expose addressable child sessions, so
  Gentd rejects child-session messaging instead of inventing correlations.
- Release hardware still must exercise the packaged runtimes and a downloaded
  model on every supported desktop target.

Any provider protocol change starts by reading the corresponding native driver,
then extending Gentd's normalized contract and tests. It never authorizes an
app-side provider bypass.
