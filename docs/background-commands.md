# Background commands

A shell command that keeps running detached from its turn (Claude `run_in_background`, a Codex
unified-exec terminal still open when its turn ends) is `toolActivity{phase:"background"}` for its
own `toolUseId`, instead of `completed` at launch. It is live work: count it as running, and keep it
open past its turn's `terminal` (it is never settled `failed` by the terminal). Its end is one later
`toolActivity` for the same `toolUseId`, scoped to the turn that launched it: `completed`, `failed`,
or `interrupted` when Stop or Send now ended it. Stop and Send now end every background command of
the run (Claude `stop_task` per task, Codex `thread/backgroundTerminals/clean`, Gent local
`session/cancel` for each ACP session with a background command). Example, Claude:

```json
{"type":"toolActivity","turnId":"T","activity":{"toolUseId":"toolu_1","toolName":"PowerShell","phase":"started"}}
{"type":"toolActivity","turnId":"T","activity":{"toolUseId":"toolu_1","toolName":"PowerShell","phase":"background"}}
{"type":"terminal","turnId":"T","phase":"ready"}
{"type":"toolActivity","turnId":"T","activity":{"toolUseId":"toolu_1","toolName":"PowerShell","phase":"completed"}}
```

While one runs, the run stays live (it keeps being polled and is not reaped as idle) and still accepts the next prompt.
`run_live_status` counts it as live command work, so Stop stays offered and an interrupt of an ended
turn reports `backgroundWorkRunning: true`. An end that a later run of the same conversation reports
is filed under the launching run and turn.

Claude reports `session_state_changed` `idle` as soon as the launching turn's `result` is written,
while the command still runs. When the command ends, Claude writes `task_updated` and
`task_notification` on its own, then `running`, a continuation turn about the result, and `idle`
again; after `stop_task` it writes the end and `idle`. The Claude driver therefore reports the
session idle only when no background command of the run is open, so the run is polled until that
last `idle` and the end and continuation are recorded when Claude emits them.

Only a command that started executing becomes `background`. A Codex command whose approval
request is still unanswered when its turn ends never ran: it ends with that turn, `interrupted`
when Stop ended the turn, otherwise `failed`. An approved command still running then is
`background` as above.

An open sub-agent is live work the same way. Claude keeps its session `running` until its
background agents end. Codex reports its root thread idle while a spawned thread still works, so
the Codex driver reports the session idle only when no command and no spawned thread of the run is
open. A sub-agent's permission request or question made while no turn is open is recorded when the
provider sends it, and its end settles the sub-agent then.

Every background command gets an end, whatever stops it. The store owns one rule
(`gent-store` `background_tools`): a toolUseId whose latest `toolActivity` phase is `background` is
open. Startup recovery ends every open one recorded by an earlier host epoch as `failed`, with a
`gentdRestarted` notice on its turn, for every provider. A Gent local runtime that is stopped or
replaced (a failed or interrupted turn, a drain failure, a relaunch for a new selection) ends its
conversation's open ones at release: `interrupted` when a Stop caused it, otherwise `failed`.

Gent local (Claurst) reports a background command over ACP as a tool call still `in_progress` when
`session/prompt` answers, then one later `tool_call_update` for the same `toolCallId`: `completed`,
`failed` with `rawOutput.status` `cancelled` (`interrupted`), or another `failed` (`failed`). Gentd
keeps reading Claurst's stdout after the terminal while one is open, records the end under the
launching turn, and keeps the runtime from being reaped. A Stop with no running turn sends
`session/cancel` for the stopped run's sessions that have one (never another conversation's);
Claurst never re-arms a cancelled session, so the next prompt opens a fresh one. When Claurst exits
with one open, Gentd ends it `failed`.
