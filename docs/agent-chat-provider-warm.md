# Provider warm-up while the user types

Clients that negotiate `agent-chat-provider-warm-v1` may send `warmProvider` when the user focuses
or starts typing in the composer of an existing conversation:

```json
{"type":"warmProvider","body":{"requestId":"r-1","conversationId":"c-1","runId":"run-1"}}
```

`runId` is the conversation's current run as the client last read it. Gentd answers with exactly
one correlated result and never an error for an expected refusal:

```json
{"type":"providerWarm","body":{"requestId":"r-1","conversationId":"c-1","runId":"run-1","outcome":"warming"}}
```

| `outcome` | Meaning |
| --- | --- |
| `warming` | A provider process was started for this run. |
| `alreadyWarm` | This run already owns a live process; its idle clock was refreshed. |
| `turnActive` | A turn of this conversation is active or a prompt is pending. |
| `superseded` | `runId` is no longer the conversation's current run (selection switched). |
| `unsupportedProvider` | The run's provider host does not warm processes. |
| `poolFull` | The provider host limit is reached and every owned process is busy. |
| `unavailable` | The provider could not start (missing binary, auth, sandbox); the next prompt starts normally and reports the real error. |

The intent carries no receipt and writes no prompt, turn, transcript, activity or provider-session
fact. Repeating it is idempotent. Remote viewers forward it through the owner's existing
`gentd-intent` relay.

## Lifecycle

- Codex runs `initialize` and `thread/start` (or `thread/resume`) with no turn; Claude is spawned
  with its launch flags and receives no input.
- Claurst answers `warming` once its owner accepts the request, then starts the run's local
  runtime (llama-server loads the model, `claurst acp` launches) and completes ACP `initialize`
  while no prompt is active. It never downloads a model and never opens `session/new`, so the
  first prompt still renders history into a fresh session. The runtime is the same one a prompt
  would start and stays until a prompt changes its selection or fails; the pool rules below are
  for Claude and Codex. A failed warm-up is logged and the next prompt starts normally.
- The warm process is bound to one conversation, run, provider, model, effort, mode, workspace,
  MCP configuration and permission policy revision, and is launched through the same sandbox
  permission binding as a prompt. The first prompt for that run adopts it only if all of these
  still match; otherwise the warm process is stopped and the prompt launches normally.
- Pre-prompt provider facts (such as the provider session identity) are held in memory and
  recorded only when a prompt adopts the process, so an unused warm process leaves no trace.
- Idle processes are reaped every 15 seconds: never-prompted ones after 2 minutes, ones that
  served a turn after 10 minutes, and any run that is no longer its conversation's current run
  immediately. A selection switch also reaps immediately.
- Warm processes count toward the provider host limit and are drained on shutdown. Nothing is
  recovered for them after a restart.
