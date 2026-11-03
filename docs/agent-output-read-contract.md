# Agent output reads

Gentd projects provider-exposed text through one canonical transcript mapping.
Assistant answers, thinking, plans and notices retain distinct transcript kinds.
Tool input and lifecycle use `ToolActivity`. Tool results also produce a
`ToolOutput` activity containing `toolUseId`, bounded `text` and `isPartial`.
Both public provider lifecycles and ACP use that shared mapping. Clients render
canonical facts rather than parsing provider frames.

Codex approval metadata may omit the operation carried by the corresponding
started item. The driver correlates bounded normalized previews by exact
thread/turn/item before committing the permission request and its digest. Native
request fields override cached fields; missing detail never comes from another
scope. Recent live-item context survives a newer turn until item completion or
finite eviction. Existing committed permission requests are never rewritten.

ACP tool-call updates replace their content collection. Running updates retain
their tool phase while replacing displayed result content; explicit empty content
clears a prior result. Structured raw results remain JSON text. Error updates
become provider failures containing exposed error text rather than raw metadata.

Codex completed reasoning can supply text/content and summary independently.
The turn driver tracks whether each channel streamed using thread, turn and item
identity. Completion emits only unstreamed channels and never removes repeated
equal text from unrelated items or parts. Both channels still use the existing
`Thinking` wire vocabulary; distinct summary presentation remains separate work.
Each native turn explicitly requests provider-exposed `summary: auto`. This
does not guarantee the provider emits reasoning or explain earlier zero counts.
Codex plan updates preserve explanation and step status in canonical plan text.
Child token/context totals do not replace the root turn's totals.

## Historical tool results

Older normalized batches retained `ToolOutputDelta` in their immutable lifecycle
but omitted output activities. Reads derive an output only when the complete
normalized batch agrees with its original lifecycle event identity, host epoch,
conversation, run, turn and lifecycle payload, and the batch has no materialized
activity. Tool identity always comes from that lifecycle fact.

Activity pages merge derived results at their original lifecycle event cursors.
Projection pages and tails reinterpret the existing lifecycle projection as an
activity at its existing projection cursor and `sourceEventId`. Derived output
includes `sourceLifecycleEventId`; its projection envelope also includes
`derivedFromLifecycleEventId`. New materialized output activities are not derived
again. The ledger, projection rows and accepted cursor identities are unchanged.

This widens the read interpretation of historical lifecycle projections. A client
that already accepted their cursors must rehydrate from authoritative transcript
and activity pages when reconnecting after an upgrade; following only after an old
last cursor cannot reveal previously omitted presentation content. No client may
infer a tool ID from transcript text or manufacture replacement durable facts.

## Authentication status

An active authentication status may include `challengeId`. It identifies only the
exact provider challenge on the responding owner; cancellation still requires
that exact identifier. Missing IDs remain readable, and terminal states omit it.
Login monitoring checks authoritative native authentication status at a bounded
cadence even while the login process remains alive, stops the owned process after
confirmed authentication, and settles probe timeouts at the login deadline.
Monitors are bound to provider and challenge so an earlier attempt cannot monitor
a replacement. Terminal settlement clears the challenge.

The account-browser method currently opens the browser on the owner device.
There is no remote browser URL/device-code handoff in this contract. A successful
status or cancellation regression does not prove an account-browser sign-in flow.
