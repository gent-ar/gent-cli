# Ordinary authority shutdown test matrix

This matrix is the acceptance boundary for composing the private Claude/Codex
Ask/Plan lifecycle. It does not enable authority, add recovery snapshots, or
change the hard-observer daemon. The daemon must derive recovery from durable
ledger facts and leave clients to reread cursor-bounded pages after reconnect.

## Existing unit evidence

| Contract | Existing coverage | Composition gap |
| --- | --- | --- |
| One private owner call per tick and one-slot backpressure | `private_lifecycle_loop_tests` | Keep true through daemon cancellation. |
| Shutdown order is wake, interrupt, drain wake, terminate, drain wake, kill | `private_lifecycle_loop_tests`, Claude/Codex supervisor tests | Drive this ordering from one daemon control loop. |
| An idle supervisor stops without recovery, launch, claim, or terminal fact | Claude/Codex supervisor tests | Prove daemon shutdown does not manufacture a wake. |
| A drained process settles only through a real persisted exit | Claude/Codex supervisor tests | Wait for settlement before daemon exit. |
| An undrained tree refuses invented terminal settlement after kill | Claude/Codex supervisor tests | Keep daemon failed closed after the drain deadline. |
| Hosts stay inactive until recovery or a committed prompt | `provider_lifecycle_host_tests` | Recovery arming must not admit new prompts first. |
| Cadence has no idle polling and serializes drives | `ordinary_lifecycle_cadence_tests` | Add a cancellation-aware wait path and recovery-ready result. |
| Routing selects only the durable provider selection | `ordinary_lifecycle_router_tests` | Couple its post-commit wake to the admission permit lifetime. |
| Local IPC can stop accepts and drain connections without task abort | `transport_shutdown` tests | Give ordinary authority the same one-way admission close. |

## Required composition tests

Implement these around a transient daemon control object paired with the
cadence. Do not persist its state; durable prompt and provider facts remain the
only recovery input.

1. **Startup fence.** The listener is not accepting ordinary chat commands
   until every selected host has completed its recovery drive. A recovery error
   leaves admission closed and starts no provider process. A shutdown before
   recovery starts stops the unopened authority directly: it must not call
   `begin_shutdown_after_recovery` on `AwaitingWake` hosts or manufacture a
   recovery wake merely to make shutdown possible.
2. **Admission linearization.** The daemon obtains one transient prompt permit
   *before* the facade begins the prompt transaction. Closing admission rejects
   every request that has not acquired that permit; it creates no receipt,
   prompt, wake, process, or provider fact. A request that already owns a permit
   may finish its one typed transaction and post-commit wake, after which its
   permit is released. Shutdown must wait for those permits before beginning
   provider drain. This proves a single linearization point instead of claiming
   an impossible ordering for a transaction already in progress.
3. **Recovered-idle shutdown.** After successful recovery, a host with no
   active provider work receives only its drain command and exits cleanly. It
   schedules no prompt wake, provider launch, recovery replay, or synthetic
   settlement. This exercises the existing `RecoveredIdle` path, not the
   deliberately rejected `AwaitingWake` path.
4. **Active shutdown.** After admission closes and outstanding prompt permits
   drain, every already-active host is interrupted once, then receives bounded
   drain wakes. A post-commit notification after closure is rejected at permit
   acquisition; it cannot become new work.
5. **Escalation fence.** A deadline produces `Interrupt -> drain -> Terminate
   -> drain -> Kill -> drain`; no two signals are queued before the required
   drain wake. A settled host does not receive a later signal.
6. **Terminal fence.** The daemon exits successfully only after each active
   host reports stopped following durable terminal facts. An owner error or an
   undrained process after kill keeps the daemon failed closed and does not
   write a terminal fact itself.
7. **Transport drain.** Shutdown first stops accepting new IPC connections,
   closes ordinary prompt admission, lets permit-owning requests finish or fail
   at their typed boundary, and closes subscriptions without claiming a
   replacement cursor or snapshot.
8. **Reconnect.** A disconnected client reopens local IPC, rereads immutable
   pages, and resumes its acknowledged event cursor. This test must assert that
   no recovery snapshot is serialized or consumed.

## Test mechanics

- Use paused Tokio time for escalation deadlines; do not sleep in tests.
- Drive the control object with a deterministic `recovery-ready`,
  `admission-closed`, and `all-permits-released` sequence. The control object
  is process-local; test its externally visible effects rather than persisting
  any of those booleans.
- Use fake private owners with a call log plus a ledger-backed Claude/Codex
  runner for at least one end-to-end terminal-settlement case each.
- Assert durable rows/events and process signals, not internal control state,
  except where needed to prove a one-way admission latch.
- Run each scenario for Claude and Codex. Claurst belongs to its private bridge
  CI contract and must not acquire a public credential or routing fixture here.

## Non-goals

- No always-running daemon requirement or recovery snapshot/cache.
- No task abort: do not use `JoinHandle::abort` as shutdown implementation.
- No automatic escalation without an explicit daemon-owned deadline policy.
- No release claim before strict provider evidence and the full authority gate.
