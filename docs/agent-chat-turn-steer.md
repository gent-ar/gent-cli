# Turn-bound queued steering

New clients negotiate `agent-chat-steer-turn-v1` and the selected provider's
`agent-chat-steer-turn-{provider}-v1`. The daemon advertises scoped support for
`claude`, `codex`, and `claurst`.

Claurst has no native in-turn input, so its delivery is the interrupt. The
acceptance transaction validates the same live target, then records the steer
interrupt for that turn instead of a held target. The owner interrupts exactly
that turn, which ends with terminal cause `steered` and never pauses a goal.
The prompt stays an ordinary queued prompt and is claimed ahead of the rest of
the queue, so it runs as the next turn even if the target turn ended on its own
before the interrupt landed. Claurst steers therefore never expire with
`targetExpired`; a target that already ended before acceptance is rejected with
`queuedPromptNotSteerable` like every provider.

Primary composer Steer uses `steerPromptForTurn`, gated separately by
`agent-chat-steer-enqueue-v1` plus the native turn capabilities. It carries the
same request/receipt/conversation/expected target fields, text, and optional
attachment/tool-source ID arrays. One transaction validates the live target and
creates the queued prompt, requested target, and request activity under one
receipt. It returns normal `Accepted` with the queued message's stable identity.
There is no ordinary-queue interval or second primary Steer RPC. If the target
already ended, no prompt is created. A committed uncertain retry preserves the
original request and target; it replays the receipt without resetting delivery.

The request is `steerQueuedPromptForTurn` with `requestId`, `receiptId`,
`conversationId`, `messageId`, `expectedRunId`, and `expectedTurnId`. Both expected
identities are mandatory. Clients capture them at the user's click and never
fall back to the older targetless request. Receipt and command identity include
the original target. Retrying the same command returns its original receipt;
changing its target under the same receipt is rejected. A new explicit click
after refreshing the target uses a new receipt.
An explicit same-target retry after a known rejection also needs a new receipt;
reusing the original receipt only replays the old accepted command.

Acceptance checks the conversation's durable current selection, queued message
run, exact active dispatched turn, host epoch, and ingress in one immediate
SQLite transaction. It persists the target alongside the immutable receipt and
`promptSteerRequested`. Schema v26 adds one table without rewriting existing
receipts, activities, messages, or decisions.

The provider lifecycle supplies its bound turn when claiming the prompt. An old
request cannot be claimed by a newer lifecycle binding while its target turn is
still live; that expiry rejects the steer with `targetExpired` and leaves the
message held. A steer that loses the race with its turn's end is not a failure:
the target is dropped without a `promptSteerRejected` fact and the prompt runs as
the next turn. A Claude or Codex steer never runs ahead of an older unsteered
queued prompt: it waits, that prompt runs first, and later steers join its turn
in queue order. Only Claurst, which steers by interrupt, claims a steered prompt
ahead of the rest of the queue. When Claude reads a steer written
before the result only after the turn ended, the prompt is delivered as that next
turn. Delivery must name the persisted run and turn.

Codex's native `turn/steer` includes `expectedTurnId` from its active native
session. Claude's stream transport has no native expected-turn field; the owner
fences its canonical binding and rejects late take-up after terminal instead of
promoting it into another canonical turn. Native provider acknowledgement is
separate from receipt acceptance. Receipt acceptance alone does not prove native
consumption or successful work.

`promptSteerRejected` has scope, `messageId`, `receiptId`, and `reason`. Viewers
clear only the matching request receipt, and treat `promptReleased` of the
message as the end of its steer request. `targetExpired` and `deliveryRejected`
retain the queued message, excluded from ordinary automatic queue dispatch.
An explicit new-target Steer, Edit, or Remove can recover known queued input.
After bytes were written without confirmed consumption before terminal, or
after an ambiguous process/host recovery,
`deliveryUnprovable` accompanies the existing unprovable dispatch/terminal
failure; the daemon does not automatically replay potentially delivered input.
Its notice must not claim that delivery succeeded or that the failed dispatch
remains pending. The original prompt remains durable for deliberate recovery.

The older `steerQueuedPrompt` wire intent remains compatible for old clients;
it cannot claim a request from the new targeted table. New clients use the
targeted intent exclusively. Ordinary queue dispatch excludes targeted rows;
it cannot turn a rejected steer into an automatic new turn or a resend.

Shared activity reads can contain the additive `promptSteerRejected` kind after
a targeted command. Readers must preserve/ignore unknown additive kinds rather
than reject an entire page. Older strict enum readers need an update; write
capability negotiation does not remove new facts from shared immutable history.

## Steering and selection changes

A selection or permission change made while a turn runs applies from the next
turn; it never rejects or alters the running turn. A run's selection and its
bound permission policy are immutable, and a permission request that is already
pending keeps the policy it was raised under.

A steered prompt follows the delivery of its provider:

- Claude and Codex deliver it into the running turn. It runs under that turn's
  run, selection, and permission policy, even when the user switched mode,
  model, or permission posture after the turn started. Its `promptSteered` fact
  is scoped to that run, so a client can compare the run's selection with the
  conversation's current selection.
- Claurst interrupts the running turn (terminal cause `steered`) and runs the
  prompt as the next turn under the conversation's current selection and policy.

An unstarted prompt, whether queued or still waiting for the previous turn to
settle, is not part of the running turn: it runs under the selection that is
current when it starts.

The conversation summary carries `steerDelivery` while a turn is running:
`joinTurn` when the running turn's provider delivers a steer into that turn,
`interruptThenNext` when it interrupts and runs the prompt next. It is derived
from the running run's provider through `AgentChatProvider::steer_delivery`
(Claurst steers by interrupt) and is omitted when no turn is running. With
`selectionAppliesNextTurn`, a client can say before Send now whether the steer
runs under the running turn's settings or under the new ones.
