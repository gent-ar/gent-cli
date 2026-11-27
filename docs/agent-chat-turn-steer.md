# Queued prompts, Send now and Stop

Gentd owns all three behaviours for Claude, Codex and Claurst. Clients only send intents and
render the facts below.

## A queued prompt

A queued prompt is delivered as soon as possible, in queue order:

- Claude and Codex take mid-turn input. While a turn runs, the provider lifecycle claims the
  oldest queued prompt of the conversation into that turn (`claim_for_turn` in
  `prompt_steer.rs`) and writes it: Claude as a stream-json user message with its own uuid,
  Codex as `turn/steer` with `expectedTurnId` and `clientUserMessageId`. It runs under the
  running turn's run, selection and permission policy. Gentd records `promptSteered`, scoped to
  that turn, only when the provider confirms consumption (Claude `command_lifecycle started` or
  the replayed uuid; Codex the `userMessage` item carrying the `clientId`).
- A prompt the provider returns unconsumed (Claude `command_lifecycle cancelled`, Codex a
  rejected or unconfirmed steer, or a turn that ends first) goes back to the head of the queue
  and runs next. Claude may instead run a written prompt as its own next turn; Gentd then
  records `promptReleased` and opens that turn without resending.
- Claurst has no mid-turn input; its queued prompts run as the next turn.
- A write that fails is rejected once (`promptSteerRejected`, `deliveryRejected`) and never
  claimed into a turn again; it runs as the next turn.

The conversation summary carries `queueDelivery` while a turn is running: `joinTurn` when the
running turn's provider takes queued prompts into that turn (Claude, Codex), `nextTurn` when
they wait for the next turn (Claurst). It comes from `AgentChatProvider::queue_delivery` of the
running run and is omitted when no turn runs.

Each `promptQueued` carries its own `queueDelivery`, decided in the transaction that queues it:
`joinTurn` when the conversation has a running turn (a dispatched prompt turn of this host epoch,
or a provider-started turn) whose provider takes queued prompts into it and that turn has no
Send now interrupt marker, the same marker that stops `claim_for_turn`; `nextTurn` otherwise.

## Remove

`cancelQueuedPrompt` cancels a prompt Gentd still holds in its own transaction
(`promptCanceled`). A prompt already written into a running Claude turn is marked in the
removal's transaction; the Claude lifecycle then writes
`control_request {subtype: cancel_async_message, message_uuid}` for its uuid, and when Claude
answers `command_lifecycle cancelled` the prompt ends `promptCanceled` instead of returning to
the queue. If Claude already took it in, it keeps its `promptSteered` or `promptReleased`.
Codex has no request to withdraw a steered input, so a prompt written into a Codex turn is
rejected with `queuedPromptNotCancelable`, and its `promptQueued` says `cancelable: false`.

## Send now

`steerQueuedPrompt`, `steerQueuedPromptForTurn` and `steerPromptForTurn` all mean Send now, on
every provider. The acceptance transaction records `promptSteerRequested` and a steer-interrupt
receipt for the live turn (`agent_chat_steer_target::bind`); the owner then interrupts exactly
that turn, which ends with terminal `cause: steered`. No further prompt joins that turn. The
pressed prompt is claimed first for the next turn, ahead of older queued prompts; prompts that
were written into the interrupted turn and come back unconsumed join it in queue order. With no
live turn the prompt is simply released. A Send now never pauses a goal.

Claude's interrupt is `control_request {subtype: interrupt, cancel_queued: true}`, so Claude
cancels every written, unconsumed message (`command_lifecycle cancelled`) instead of running it
itself; Gentd re-queues or withdraws each one.

The targeted requests keep their contract: `steerQueuedPromptForTurn` carries `requestId`,
`receiptId`, `conversationId`, `messageId`, `expectedRunId` and `expectedTurnId`; receipt and
command identity include the target, and a target that already ended is rejected with
`queuedPromptNotSteerable`. A prompt already written into the running turn can still be sent
now.

## Stop

`interrupt` records its receipt and, in the same store transaction
(`agent_chat_stop::stop`), withdraws every queued prompt of the conversation the provider has
not been handed: its dispatch settles, its queued turn ends `cancelled`, and
`promptWithdrawn` is appended in queue order with the prompt's `text` and attachment
references. Prompts already written into the turn are marked; when the provider returns one
unconsumed it is withdrawn the same way instead of returning to the queue. A prompt the
provider already consumed stays delivered. Queued goal continuations end with
`promptCanceled`. Withdrawn prompts are settled, so nothing delivers them again, including
after a daemon restart. The same transaction appends `interruptRequested` for every turn of
the conversation that is still open (running, launching or waiting), except a prompt's turn that
was written into the running turn. A Stop retried with the same receipt appends nothing more.
The turn then ends `interrupted` without a cause. A Claurst turn whose runtime is still
launching ends at once: the lifecycle drops the launch, which finishes in the background and
stays warm for the next prompt, and the turn settles through the unstarted-prompt path.

A Stop on a conversation with no open turn whose provider has nothing live for the run (no
session, no background work) ends the run's open background commands `interrupted` in one
transaction and answers `{"type":"error","body":{"code":"nothingToInterrupt","message":…}}`.
The conversation is settled; clients treat the code as a finished Stop, not a failure. A Stop
the provider cannot deliver while a turn is still open keeps the generic `agentChatRejected`.

## Facts

```json
{"type":"promptWithdrawn","conversationId":"…","runId":"…","turnId":"<queued turn>",
 "hostEpoch":1,"cursor":42,"messageId":"…","receiptId":"<the prompt's receipt>",
 "text":"…","attachments":[{"attachmentId":"…","displayName":"…","mediaType":"…","byteLen":5}]}
```

```json
{"type":"interruptRequested","conversationId":"…","runId":"…","turnId":"<turn Stop ends>",
 "hostEpoch":1,"cursor":43}
```

`promptSteerRejected` has scope, `messageId`, `receiptId` and `reason`
(`deliveryRejected`, or `deliveryUnprovable` after a daemon restart left a written prompt
unconfirmed). Older ledgers may also hold `targetExpired` and `selectionSwitched`.
