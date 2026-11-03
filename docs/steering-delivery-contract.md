# Steering delivery

Steering has separate durable request and provider-consumption boundaries.

- The queued-prompt command commits its receipt and a `promptSteerRequested` projection event in the same transaction. The event carries the queued prompt scope, message ID, and receipt ID. Every viewer, including one attaching later, can restore requested state.
- RPC success means the request is durable and the lifecycle wake was accepted. It does not mean the provider has consumed the prompt.
- Claude confirms uptake with the matching replay UUID. Codex confirms with the matching consumed user-message event. The lifecycle emits `promptSteered` only at consumption, or `promptReleased` when work becomes a separate turn. Gent uses its interrupt-and-dispatch path.
- Identical steer retries must retain receipt ID, host epoch, conversation, and message ID. A conflicting retry does not append another request event. Clients must reconcile across epoch changes.
- Provider output polling drains at most sixteen 4 KiB chunks per tick, reducing every chunk before reading another. This keeps memory/backpressure bounded while avoiding a 50 ms scheduler delay for every fragment of a large output frame. FIFO ordering and provider acknowledgment remain authoritative.

Driver regressions place a matching acknowledgment after a 1 MiB fragmented frame, prove no acceptance on write alone, bound work per poll, and verify exactly-once acknowledgment. Store regressions verify replayable request state and idempotency separately from delivery. Physical paired-device and real-provider timing remain release validation requirements.
