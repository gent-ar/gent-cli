# Reviewed-plan implementation

Clients that negotiated `reviewed-plan-v1` read and settle the plan a plan-mode turn proposed.
Every exchange is one request frame and one reply frame on the connection.

## Read the plan

```json
{"type":"reviewRead","body":{"requestId":"r1","conversationId":"c1"}}
{"type":"review","body":{"requestId":"r1","plan":{"planId":"p1","conversationId":"c1","sourceRunId":"run-1","sourceTurnId":"t1","revision":2,"contentDigestSha256":"<64 hex>","status":"readyForReview","content":"1. ..."}}}
```

`planId` in the request is optional; without it Gentd answers the conversation's current plan.
Only a plan with `status` `readyForReview` can be started or rejected. `planUpdated` activity facts
carry the same artifact whenever it changes.

## Start it, with or without instructions

"Start" and "Clarify & Start" are the same intent. The clarification is `instructions`, optional,
1 to 16384 bytes of text that is not blank:

```json
{"type":"startImplementation","body":{"request":{
  "requestId":"r2","receiptId":"rc2","idempotencyKey":"plan-start-rc2","hostEpoch":3,
  "policyWorkspaceId":"w1","policyRevision":2,"conversationId":"c1",
  "planId":"p1","planRevision":2,"planContentDigestSha256":"<64 hex>","parentRunId":"run-1",
  "selection":{"provider":"claude","model":"haiku","effort":"medium","mode":"agent"},
  "contextPolicy":"preserve","instructions":"Keep the public API unchanged."}}}
```

`parentRunId` is the plan's `sourceRunId`, `planRevision` and `planContentDigestSha256` are the
plan's `revision` and `contentDigestSha256`, and `policyRevision` is the workspace's current
provider-permission policy revision. In one transaction Gentd approves that exact revision, creates
the implementation run with `selection`, saves one implementation prompt on it and appends
`promptReleased{messageId}` for that prompt's turn. The prompt holds the approved plan followed by
the instructions, so the provider receives both in the first implementation turn. Its
`userMessage` lands in the same transaction. Adopt the turn from `promptReleased` as for a queued
prompt. The reply carries `implementationRunId`:

```json
{"type":"startedImplementation","body":{"requestId":"r2","result":{"implementationRunId":"...","planRevision":2,"...":"..."}}}
```

A retry with the same `receiptId` and `idempotencyKey` and an identical request returns the same
result; a retry that changes any field, `instructions` included, is refused.

A client that queued the clarification as a separate prompt after `startedImplementation` drops
that step: the instructions are already the first user message of the implementation turn. A
viewer that relays the decision to the owning device relays `instructions` with it, and the owner
puts them on its one `startImplementation`.

Do not send the clarification as an ordinary prompt while the plan awaits review. That prompt runs
as a new planning turn on the plan-mode run: the approval is then refused with
`planApprovalBlockedByActiveTurn` while it runs, and with `planNotReviewable` once it produced a
newer revision.

## Reject it

```json
{"type":"reject","body":{"requestId":"r3","planId":"p1","planRevision":2,"planContentDigestSha256":"<64 hex>"}}
{"type":"rejected","body":{"requestId":"r3","planId":"p1","planRevision":2}}
```

## Refusals

A refused `startImplementation` or `reject` is an error frame whose `code` names the reason. Match
the code, never `message`:

| code | meaning |
|---|---|
| `planApprovalBlockedByActiveTurn` | a turn on the planning run is still active |
| `planNotReviewable` | the revision is no longer `readyForReview` (superseded, approved, rejected) or its digest differs |
| `planApprovalParentNotCurrent` | `parentRunId` is not the conversation's current run |
| `planApprovalPolicyChanged` | `policyRevision` is not the workspace's current policy revision |
| `reviewedPlanRejected` | any other failure; `message` is diagnostic text |

A malformed frame, `instructions` included, is answered with `invalidReviewedPlan`. After
`planNotReviewable` or `planApprovalPolicyChanged`, read the plan and policy again and show the
current revision before the user approves.
