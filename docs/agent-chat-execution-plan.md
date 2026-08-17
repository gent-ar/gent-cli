# Reviewed-plan execution contract

This records the native-agent-chat behavior Gent must own before Flutter uses
it for live provider work. It is a future authority contract, not a capability
of the shipped observer daemon.

## One source of truth

Gent owns the reducer, durable records, permission evaluation, context boundary,
provider selection, and lifecycle facts. `gent` renders and invokes those same
commands directly in the terminal. The native app is another IPC client: it
renders Gent snapshots and sends typed user choices, but does not keep a second
plan state machine, rebuild context, select a provider, or infer execution
state. A capability unavailable to `gent` is unavailable to the native app.
The complete browse/create/prompt/follow-up/reconnect lifecycle is specified in
[the realtime agent-chat client contract](realtime-agent-chat-client-plan.md).

## Existing foundation

`AgentChatSelection` already carries provider, model, effort, and mode. A
receipt-backed `SwitchSelection` already creates an immutable child run and
freezes its inherited conversation ordinal. The current isolated authority
profile persists only intent; it does not review a provider plan, launch a
provider, or make activity live truth.

## Required reviewed-plan flow

1. A provider-normalized plan becomes an immutable durable `PlanArtifact` with
   an ID, source run/turn, revision, content digest, review status, actions,
   risks, diffs, and a permission preview. Provider raw events, provider session
   IDs, credentials, and hidden reasoning do not become plan fields. Its closed
   reducer states are draft, readyForReview, approved, rejected, superseded, and
   terminallyFailed; the review history is append-only.
2. The app requests a review snapshot by plan ID. The snapshot includes the
   plan revision, its source boundary, actions/diffs/risks/permission preview,
   supported selection choices, and the current terminal status. It is
   cursor/revision bound and cannot be inferred from displayed text.
3. `Start implementing` is a receipt-backed command for one exact reviewed
   plan revision. The user chooses provider, model, effort, and mode at this
   approval point; Gent validates that choice against the locked provider
   capability catalog rather than trusting an app-side model list. It also
   freezes the policy revision and host epoch evaluated at approval.
4. Approval creates a new immutable implementation child run. It never mutates
   the plan run or changes a provider/model in place. The child records its
   parent, selection, reviewed-plan ID/revision/digest, host epoch, and receipt.
5. A duplicate approval with the same idempotency key returns the original
   child; a changed plan revision, parent, selection, or context policy is
   rejected. Provider work starts only after the durable reservation and all
   authority, sandbox, binary-lock, and evidence checks succeed.

## Context policy

The approval dialog offers both choices below; neither deletes durable history.

| Choice | New provider input | Durable lineage |
| --- | --- | --- |
| Preserve context | Reviewed plan plus the frozen conversation history through its recorded ordinal | Child inherits that ordinal |
| Clear context and proceed | Reviewed plan and fixed system policy only; no prior conversation or provider-native session is resumed | Child records a zero inherited-history ordinal and the plan handoff digest |

"Clear context" therefore means a fresh provider context, not erasure. The
old conversation, plan, receipts, and runs remain readable and auditable. Gent
must never silently substitute a summary or retain a hidden provider session in
the clear-context path.

## Client and lifecycle requirements

- Gentd sends typed review, approval, rejection, and terminal activity facts;
  Flutter renders them and never starts a provider or owns alternate logic.
- The terminal UI maps its review action to the same IPC command. `/login`,
  review-plan, and approval controls all use the negotiated daemon capability.
- Plan mode stays Plan after any permission approval. Selection approval does
  not escalate permissions, and Autonomous/Bypass still need enforced sandbox
  containment for provider execution.
- If the selected provider changes, the implementation run is a child with the
  same immutable rules. Context preservation is provider-neutral normalized
  history; a provider-native resume token is never copied across providers.
- Every plan approval, clear-context choice, provider change, and launch result
  has a durable receipt and an explicit terminal result even if the provider
  never acknowledges it.

## Delivery gate

Before advertising this flow, add types, protocol frames, SQLite records,
reducer tests, receipt/idempotency/epoch and policy-fence tests, cancellation
and provider-drain tests, cursor-resumable lifecycle facts, native-app IPC
fixture coverage, and redacted live provider evidence. The observer daemon must
keep the capability absent until that work is approved.
