# Flutter Gentd handoff

Flutter is a Gentd IPC client. It starts or locates the installed Gent pair, connects to the
private data-directory endpoint, negotiates capabilities, and renders Gent-owned durable state.
It never starts Claude, Codex, Claurst, llama.cpp, MCP, Git, or a second ledger writer.

## Connection

Start the packaged daemon with `--standalone-authority` and the app-generated MCP configuration
path. On macOS and Linux the endpoint is `<data-dir>/gentd.sock`; Windows uses the
data-directory-derived named pipe. The wire format is UTF-8 JSON framed by a four-byte big-endian
length. Protocol version `1` is the current range.

For every connection, send `hello`, require `negotiated`, and use only the
returned capability intersection. On a disconnect, epoch change or expired
cursor, reconnect, negotiate, reload bounded durable pages, then continue from
the acknowledged cursor.

An in-memory view is disposable and non-authoritative. If a cursor is not accepted, reload from ordinal/cursor zero and replay immutable facts.

## Unreadable records

A stored projection record this Gentd cannot decode never stops a conversation from being read or
followed. `followConversation` delivers it in place, in cursor order, as
`{"type":"delta","body":{"requestId":…,"delta":{"type":"unreadableRecord","body":{"cursor":
{"value":11004},"recordKind":"transcript|activity|lifecycle|summary|unrecognized"}}}}` and keeps
streaming the records after it. `conversationSnapshot` serves every readable record of its tail
and names the unreadable ones in `snapshot.unreadable: [{"cursor":{"value"},"recordKind"}]`
(cursor order, empty when every record decodes). `conversationWindow` carries the same delta in
`carried`. Advance the cursor past an unreadable record; it stays unreadable, so re-reading it
changes nothing. There is no `resyncRequired` projection delta and no `projectionResyncRequired`
error: a follow that cannot read the ledger itself ends with
`{"type":"error","body":{"code":"projectionFollowFailed",…}}`.

Follows (`followConversation`, turn follow, subscriptions) are driven by ledger commits: Gentd
wakes every follower when a write to its ledger connection commits, so a delta reaches the
client as soon as the fact is durable, with no polling interval.

Status comes from the durable turn and the pending-permission row. A pending permission from a
background subagent reports `attention: required` with no open turn; a turn Claude or Codex
started on its own is an ordinary open turn (`processing`). A run whose prompt turn settled while
Claude still runs background work reports `processing: idle`, with `subagentWork` showing any
background subagent that is still open. `interrupt` with no open turn still reaches the live
provider session: Stop sends Claude's protocol interrupt, which ends its background agents.
Send that Stop without `expectedTurnId`. An `interrupt` whose `expectedTurnId` names a turn that
already settled is rejected, with no receipt, as
`{"type":"error","body":{"code":"interruptTurnEnded","message":…,"interrupt":{"conversationId",
"runId","turnId","turnPhase","backgroundWorkRunning"}}}` (`agent-chat-interrupt-turn-ended-v1`);
when `backgroundWorkRunning` is true, offer "stop background work" as an interrupt without
`expectedTurnId`.
Gentd never resends a failed prompt.

## Queued prompts, Send now and Stop

Gentd owns all three for every provider (`docs/agent-chat-turn-steer.md`); render the facts.
The summary field `steerDelivery` is gone. While a turn runs the summary carries
`"queueDelivery":"joinTurn"` (Claude, Codex: a queued prompt joins the running turn) or
`"queueDelivery":"nextTurn"` (Claurst: it runs as the next turn); it is omitted when no turn
runs. Label each queued prompt from its own `promptQueued` ("Joining this turn" or "Next turn").
Gentd decides it when the prompt is queued: `joinTurn` when a turn is running whose provider
takes queued prompts into it (Claude, Codex) and no Send now interrupt is pending on that turn;
`nextTurn` otherwise (Claurst, no running turn, or a pending Send now). `turnStarted` carries no
`queueDelivery`; Claude does not emit `turnStarted` for a turn that a prompt opens.

A selection switch moves every prompt that has not started onto the new run. When no turn of the
conversation is running, the new provider's readiness decides each moved prompt again, as for a
new prompt: one for a provider that is not installed gets `promptHeld{reason:"providerInstall"}`
on the new run (a `send` prompt; the install card answers it) and runs on that provider once the
install settles. While a turn is running, moved prompts stay ready to join that turn.

```json
{"type":"promptQueued","conversationId":"c","runId":"r","turnId":"t2","hostEpoch":1,"cursor":42,
 "messageId":"m2","queueDelivery":"joinTurn","cancelable":true}
```

A `promptQueued` recorded before these fields existed has no `queueDelivery`; fall back to the
summary for it. Offer Remove (`cancelQueuedPrompt`) only when `cancelable` is true (absent on
older facts means true): it is false for a prompt joining a Codex turn, which Codex takes in
when Gentd writes it and has no request to withdraw. A Claude prompt stays removable until
Claude consumes it: the reply means the removal is durable, and the row ends with
`promptCanceled` (no composer return) once Claude hands it back, or with `promptSteered` /
`promptReleased` when Claude had already taken it. A prompt that can no longer be removed is
rejected with `queuedPromptNotCancelable`. Removing one Claude took in appends, in its transaction,
`promptCancelRequested{turnId:T,messageId,receiptId}` once: show "Removing" until the row ends.

Queue (`queuePrompt`) while turn T runs:

- Claude, Codex: `promptQueued` → `promptSteered{turnId:T,messageId,receiptId:<the prompt's
  receipt>,transcriptCursor}` once the provider takes it in. If T ends first, the prompt runs
  next: `promptReleased` then its own turn (Claude may start that turn itself; same facts).
- Claurst: `promptQueued` → `terminal` of T → `promptReleased` → its own turn.

Send now (`steerQueuedPrompt`, `steerQueuedPromptForTurn`, `steerPromptForTurn`) on prompt P
while T runs, on every provider: `promptSteerRequested{turnId:T,messageId:P,receiptId}` →
`terminal{turnId:T,phase:"interrupted",cause:"steered"}` → `promptReleased{messageId:P}` → P's
turn, ahead of older queued prompts; prompts that were joining T and were not consumed join P's
turn (`promptSteered`). With no running turn: `promptSteerRequested` (scope: P's own turn) →
`promptReleased`. A prompt shown as joining can still be sent now.

Stop (`interrupt`): in the interrupt receipt's own transaction, for each queued prompt Gentd
still held, in queue order, `terminal{turnId:<its queued turn>,phase:"cancelled"}` then

```json
{"type":"promptWithdrawn","conversationId":"c","runId":"r","turnId":"t-queued","hostEpoch":1,
 "cursor":42,"messageId":"m","receiptId":"receipt-of-the-queue-request","text":"typed text",
 "attachments":[{"attachmentId":"a1","displayName":"diagram.png","mediaType":"image/png",
 "byteLen":5}]}
```

In the same transaction, after those, Gentd appends `interruptRequested` for T (and for any other
open turn of the conversation), so every device can show that Stop was accepted before the
provider answers:

```json
{"type":"interruptRequested","conversationId":"c","runId":"r","turnId":"T","hostEpoch":1,"cursor":43}
```

Then T ends `terminal{phase:"interrupted"}` with no `cause`. A Gent local (Claurst) turn whose
runtime is still launching ends at once rather than after the launch. A prompt already written into T
gets its `promptWithdrawn` when the provider returns it unconsumed (Claude cancels it at once,
Codex at the end of T), so it can arrive after the receipt; a prompt the provider consumed keeps
its `promptSteered` and is not withdrawn. Queued goal continuations end with `promptCanceled`.
Nothing withdrawn is ever delivered, also after a daemon restart. On `promptWithdrawn`, drop
the queued bubble and put `text` and `attachments` back into the composer, in the order of the
prompts' `promptQueued` facts; the attachment ids stay valid for a new prompt.

A turn ending `interrupted`, `cancelled` or `failed` settles, in the terminal's transaction and
before the `terminal`: its pending decision, each tool still `started`/`waitingPermission` (child
tools too) as `toolActivity{phase:"failed"}`, and each sub-agent it started that has not ended as
`workPhase{kind:"subagent",phase:"interrupted"}` (after that child's decision settles). No later
tool fact of the turn, nor `workPhase` of an ended sub-agent, is recorded. A `ready` turn settles
only its own decisions; sub-agent tools and decisions stay open. A question (`AskUserQuestion`) is
answered by an allow carrying its answers as `input`; none is `agentChatPermissionInputInvalid`.

Background commands (`toolActivity{phase:"background"}` and its later end): `docs/background-commands.md`.

## Context cleared and compaction

Every switch with `contextPolicy: "clear"` appends, in the switch transaction, on the new run:

```json
{"type":"contextCleared","conversationId":"c","runId":"<new run>",
 "turnId":"context-cleared:<receipt>","hostEpoch":1,"cursor":44,"command":"clear"}
```

`command` is `"clear"` only when the clear came from `invokeCommand` resolving to `clearContext`
(`/clear` or its alias `/reset`). It is absent for every other clear, such as a `switchSelection`
the client sends with `contextPolicy: "clear"` for "Start Implementing with clear context". Show
the user's `/clear` from this fact; do not infer it from the switch.

A Claude `/compact` records the `providerContextCompacted` diagnostic, its notice and the
activity fact `contextCompacted` (scope only) but no `contextUsage` (Claude reports none until its
next request); a Gent local summary appends `contextCompacted` too. Context usage is unknown from
the latest `contextCompacted` until a later `contextUsage`; snapshots and windows carry it.

Claude's built-in commands (such as `/compact`) resolve from Gentd's own table while Claude's
listing loads; `commandCatalogLoading` only answers a provider skill asked for before then. When
a listing settles (loading to listed or failed) every projection follow receives, at its current
cursor, `{"type":"catalog","body":{"cursor":C,"catalog":{"catalogId":"commands","selectedId":null,
"records":[]}}}`: re-read `readCommandCatalog` instead of polling.

## What a conversation changed in its workspace

Gentd records what a conversation changed in its workspace folder, whichever tool wrote it (an
edit tool, a shell heredoc, `mv`, a generator). When the first prompt of a conversation is
admitted (`sendPrompt`, `queuePrompt`, `sendPromptWithTools`, `queuePromptWithTools`), before the
provider can run, Gentd snapshots the workspace as a Git tree object (a private index file; the
user's index and working tree are not touched). After each `terminal` of that conversation it
appends, on the terminal's run and turn:

```json
{"type":"workspaceChanges","conversationId":"c","runId":"r","turnId":"T","hostEpoch":1,
 "cursor":51,"files":[{"path":"lib/main.dart","added":3,"removed":1},
 {"path":"notes.md","added":12,"removed":0}],"truncated":false}
```

- `files` is cumulative from that first-prompt snapshot to the workspace right after the
  terminal, not this turn alone: the latest `workspaceChanges` of a conversation replaces every
  earlier one. A file changed and then restored drops out.
- `added` and `removed` are `git diff --numstat` line counts. `path` is relative to the
  conversation's workspace folder, with `/` separators. Only paths under that folder are listed.
- Edits that existed before the first prompt are not counted. Untracked files count; files Git
  ignores, everything under `.git` and binary files are never listed.
- At most 500 files are listed, in Git's path order; `truncated: true` means more changed.
- A folder that is not inside a Git work tree gets no snapshot and no `workspaceChanges`, ever.
  The same goes for a conversation whose first turn did not come from one of those four intents
  (the snapshot is taken only there), and for a turn that ended while Gentd was not running. A
  turn whose snapshot fails (Git missing, a 20-second Git deadline, more than 1 MiB of numstat
  output) gets no fact; the next terminal reports the cumulative state again.
- The fact is recorded shortly after the `terminal`, by one daemon owner, so a queued prompt that
  starts at once may already have written files that it includes; the next terminal corrects it.

## Reviewed-plan implementation

Approving, approving with instructions ("Clarify & Start") and rejecting a plan, with their typed
rejections, are specified in `docs/reviewed-plan-implementation.md`. How a permission or question
was answered is `decisionSettled.outcome`, specified in `docs/permission-decision-outcome.md`.

## Missing workspace folder

When the folder a request needs does not exist or is not a directory, Gentd answers
`{"type":"error","body":{"code":"workspaceUnavailable","message":…,"workspace":{"path"}}}`.
`path` is the client's path for `createConversation` and `resolveRequest`, and the stored
canonical path for everything else. It answers every prompt admission (below) and provider-native
slash commands before a receipt is recorded, workspace
Git `resolveRequest`, `statusRequest` and `subReposRequest`, and permission-policy `current` and
`save` for a known workspace. Conversation reads (detail, transcript, snapshot, follow) never
check the folder, so history stays readable.

## Sandbox the provider cannot enforce

Prompt admission is `sendPrompt`, `queuePrompt`, `sendPromptWithTools`, `queuePromptWithTools`,
`steerPromptForTurn`, `steerQueuedPrompt`, `steerQueuedPromptForTurn`, `continueFromSavedHistory`
and `sendToConversation` (checked against the target conversation), plus `warmProvider` (before
any provider spawn) and `createLinkedConversation` (checked against the parent conversation, before
the child is created). One check answers all of them, missing folder first. When the conversation's
containment is `sandboxed` and its selected provider has no OS sandbox on this host, Gentd answers
before any receipt, dispatch or ledger write with
`{"type":"error","body":{"code":"sandboxUnavailable","message":…,"sandbox":{"provider":
"claude|codex|claurst","containment":"sandboxed"}}}`. Render from `provider` and `containment`;
`message` is diagnostic text. The provider launch runs the same check, so a conversation whose
containment changed after admission settles its turn with the transcript `failureCode`
`sandboxUnavailable`. Clients need no permission-policy round-trip before sending: Gentd reads the
current policy and containment itself at admission and at launch.

Permission-policy `save` takes `requestId`, `policy` and `bypassConsent` only. A policy's mode
never decides whether a provider can launch; containment is per conversation and is refused at
conversation-config `save` on a host with no sandbox.

## Typed turn failures

Every provider failure (Claude, Codex, Claurst) reaches the turn's transcript as a `notice` whose
`failureCode` is typed; read the code, never the text. Codes and event-id prefixes:
`providerAuthRequired` (`provider-auth-required:`), `providerRateLimited`
(`provider-rate-limited:`), `providerContextLimit` (`provider-context-limit:`),
`providerOutputLimit` (`provider-output-limit:`), `providerOverloaded` (`provider-overloaded:`),
`providerFailed` (`provider-failed:`, every other provider or lifecycle failure). A rate-limited
notice carries `resetsAt` (Unix seconds) on the transcript event itself when the provider
reported one; the field is omitted otherwise. Pages, the snapshot tail and projection deltas
carry the same fields. Codex `serverOverloaded` maps to `providerOverloaded`; Claude has no
overload signal Gent has verified, so its server errors are `providerFailed`.

## Side questions and model downloads

A side question that settles appends one activity fact to its conversation's projection:
`{"type":"sideQuestionSettled","conversationId","runId","turnId","hostEpoch","cursor",
"sideQuestionId","askReceiptId","status":"answered|failed|cancelled","answer","failureCode",
"failureReason"}`. A `failed` fact carries `failureCode`: `providerBusy` (the provider was
running a turn), `providerUnavailable` (no runtime or executable could answer) or
`providerFailed` (the provider ran and failed). Every `failed` side question carries
`failureCode`, in this fact and in the `asked`, `cancelled` and `sideQuestions` records
(`{"sideQuestionId","conversationId","question","status","answer","failureCode","failureReason",
"createdAtUnixMs"}`, `failureCode` null unless `failed`). `failureReason` is diagnostic text only;
render the code. Claurst answers side questions only between turns:
its catalog `/btw` reports `availability.blockedWhileTurnActive: true`, and an ask while a turn
runs is rejected before acceptance as `{"type":"error","body":{"code":
"sideQuestionBlockedByActiveTurn","message":…}}`. Between turns Claurst answers with the
conversation's selected local model, loading it first when it is not loaded, so the first answer
after a restart takes the model's load time. Claude and Codex answer during a turn.
`askReceiptId` is the `receiptId` the client sent in `askSideQuestion`, so a fact that arrives
before the `asked` reply is matched by that id; the reply's `sideQuestionId` is not needed. It is
an immutable projection fact, so a restore from pages sees settled answers and followers need no
side-question poll.

While a prompt is held for a model download, the held prompt's own waiter appends
`{"type":"modelDownloadProgress",…scope,"modelId","messageId","receiptId","downloadedBytes",
"totalBytes","phase":"downloading|verifying|ready|failed"}` to that prompt's conversation. A
follower never becomes a download holder. It is emitted on every phase change and at most once
per hundredth of `totalBytes` while downloading; `verifying` means every byte arrived and the
digest is being checked. `ready` is followed by `promptReleased`, `failed` by `promptCanceled`;
a prompt the user cancels ends with `promptCanceled` alone.

## Opening a long conversation from a turn boundary

With `agent-chat-projection-window-v1`, a client restores a long conversation from a turn boundary
instead of folding from cursor zero:

1. `conversationTurnCursorsRequest{requestId, conversationId, beforeCursor?:{value}, limit<=200}`
   answers `conversationTurnCursors{requestId, conversationId, turns:[{turnId, runId,
   firstProjectionCursor, firstTranscriptCursor, firstActivityCursor}], nextBeforeCursor}`,
   newest first. Page older turns with `beforeCursor: nextBeforeCursor`.
2. Pick a boundary `S = firstProjectionCursor` and send
   `conversationWindowRequest{requestId, conversationId, fromCursor:{value:S}}`. The answer
   `conversationWindow{requestId, conversationId, fromCursor, lastTurn, carried}` names the turn
   before `S` (`lastTurn`: `turnId`, `runId`, `phase`, …) and carries the immutable projection
   deltas below `S` that a fold starting at `S` still needs: every fact of each turn still open,
   of each turn holding a pending decision (with its full request), of each turn that recorded
   subagent or command work the run's live status still counts as running (the same work that
   sets `subagentWork`, `commandWork` and `backgroundWorkRunning`), even when that turn settled,
   and of each turn that also has facts at or after `S`; the latest `goalUpdated`, `planUpdated` and `summary`; the latest
   32 `sideQuestionSettled`; and the conversation's linkage facts. Queued prompts, steer requests and rejections, held admissions
   and queued questions are facts of those turns, so they arrive here.
3. Fold `carried`, then `followConversation{afterCursor:{value:S-1}}` to head. Permission policy
   and grants stay on their own read.

Every read is index-backed. All are independent of history length except the open-work read,
which folds each run's subagent and command lifecycle facts exactly as the live status does. More than 4096 carried facts is
rejected as `projectionWindowTooLarge`; restore from cursor zero then.

`lookupReceipt` for an accepted prompt also returns `prompt.delivery`
(`queued|awaitingReadiness|awaitingProvider`; a stored queue intent reads
`queued` and a stored send reads `awaitingReadiness`).

## Standalone capability surface

| Need | Capability |
| --- | --- |
| Conversations, selection, prompts, queue, interrupt | `agent-chat-intents-v1` |
| Conversation list and detail | `agent-chat-conversations-v1` |
| Normalized transcript pages | `agent-chat-transcript-v1` |
| Exact live-turn stream | `agent-chat-turn-follow-v1` |
| Permissions | `agent-chat-permissions-v1` |
| Approve a command pattern (`approveCommandPattern` + `commandPattern`; policy `allowedCommands`) | `agent-chat-permission-command-grants-v1` |
| Read a receipt's state by `receiptId` (`lookupReceipt` → `receiptState`) | `agent-chat-receipt-lookup-v1` |
| Typed `interruptTurnEnded` for an interrupt of a settled turn | `agent-chat-interrupt-turn-ended-v1` |
| Turn boundaries and a window restore (`conversationTurnCursorsRequest`, `conversationWindowRequest`) | `agent-chat-projection-window-v1` |
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
