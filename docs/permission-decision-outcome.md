# How a decision was settled

Every `decisionSettled` activity fact that Gentd records for a permission or question it relayed
carries `outcome`:

```json
{"type":"decisionSettled","conversationId":"c1","runId":"run-3","turnId":"turn-12","hostEpoch":1,"cursor":251,"decisionId":"decision-12","outcome":"skipped"}
```

| outcome | when |
|---|---|
| `allowed` | the user approved it (any approve response), or answered a question with its answers as `input` |
| `denied` | the user denied a permission |
| `skipped` | the user answered a question (a tool that needs a user response, such as `AskUserQuestion`) with `deny` |
| `cancelled` | it ended without the user's answer: its turn or child ended, Stop, a stale or undeliverable answer, a broken provider pipe, or Gentd restart recovery |

`outcome` is absent on facts recorded before this field existed and on a settlement the provider
reported itself (Gent local's `decision_settled`). Treat an absent `outcome` as unknown, never as
`cancelled`. The matching `decisionPending` fact still carries the request, so a client can render
"Skipped" for a question and "Denied" for a permission from `outcome` alone.
