# In-turn delivery

In-turn delivery has separate durable request and provider-consumption boundaries.

- A queued prompt is claimed into the running turn by the provider lifecycle, not by a client
  request. The claim records which turn it was written into, so the turn status keeps naming
  the running turn while the written prompt waits.
- RPC success of a queue or Send now intent means the request is durable and the lifecycle wake
  was accepted. It does not mean the provider has consumed the prompt.
- Claude confirms uptake with the matching replay uuid or `command_lifecycle started`. Codex
  confirms with the matching consumed user-message event. The lifecycle emits `promptSteered`
  only at consumption, or `promptReleased` when the prompt becomes a separate turn, or
  `promptWithdrawn` when a Stop withdrew it.
- Identical Send now retries keep receipt ID, host epoch, conversation and message ID. A
  conflicting retry does not append another request event. Clients must reconcile across epoch
  changes.
- Provider output polling drains at most sixteen 4 KiB chunks per tick, reducing every chunk before reading another. This keeps memory/backpressure bounded while avoiding a 50 ms scheduler delay for every fragment of a large output frame. FIFO ordering and provider acknowledgment remain authoritative.

Driver regressions place a matching acknowledgment after a 1 MiB fragmented frame, prove no
acceptance on write alone, bound work per poll, and verify exactly-once acknowledgment. Store
regressions (`crates/gent-store/tests/agent_chat_queue_send_now_stop.rs`) verify in-turn claims
in queue order, Send now ordering and the Stop withdrawal. The behaviour is described in
`docs/agent-chat-turn-steer.md`.
