# Gent-native multi-agent orchestration plan

## Purpose and boundary

Gent will orchestrate Claude, Codex, the private Claurst bridge, and approved
custom harnesses as one daemon-owned system. This plan takes the useful
meta-harness lesson from an external audit—catalogued harness capabilities,
isolated workers, and independent review—but defines a new Gent contract. It
does not import another project's code, configuration language, endpoints, or
credentials.

`gentd` remains the sole composition root and ledger writer. `gent` and the
native app are equal protocol clients: they render graph snapshots and submit
typed commands. Neither launches a worker, parses provider output, allocates a
worktree, decides scheduling, or writes orchestration state. Claurst remains a
credential-free public port with its implementation and evidence in private
app-owned code.

The default daemon remains hard observer. It advertises none of the commands
or capabilities below until the authority and evidence gates are met.

## Product model

An immutable `HarnessProfile` catalog identifies an approved worker shape:
profile ID/revision, provider kind, locked executable or private bridge class,
declared input/output/interrupt/resume capabilities, compatible selection
range, and containment requirement. A profile is catalog data, not an arbitrary
shell command, provider URL, or client-supplied configuration. Custom profiles
are installed only through a separate signed, receipt-backed administrator
authority; their launch adapter is a daemon port and their output is normalized
into the same facts as public drivers.

`TaskGraph` is a durable, typed graph bound to one conversation, root run,
goal revision, policy revision, host epoch, and workspace/repository identity.
Each `TaskNode` has an immutable ID, role, exact harness-profile revision,
selection, input artifact digests, dependency IDs, worktree policy, retry
budget, and terminal result reference. Edges are dependency-only and must be
acyclic. The graph is never reconstructed from provider text.

Roles are declarative (`planner`, `implementer`, `researcher`, `reviewer`, or
a catalogued custom role) and do not grant capability. Every selected profile
must independently pass the locked-binary/private-bridge, sandbox, provider
evidence, workspace lease, permission, and policy checks at dispatch time.
Autonomous affects permission evaluation only; it cannot create workers,
override a goal, bypass review, or relax those fences.

## First-class commands

`orchestration-v1` is a negotiated IPC extension with bounded typed frames,
request correlation, receipt/idempotency keys, expected revision, host epoch,
and policy/goal binding. The current foundation exposes only graph persistence,
and only from the explicit agent-chat persistence profile; the default daemon
does not advertise this capability.

- `Fanout`: `gent orchestration fanout --graph-json FILE` reads one strict JSON
  `FanoutRequest`, then atomically creates its daemon-owned graph and requested
  nodes. It is neither a prompt template nor a provider instruction.
- `CrossReview`: `gent orchestration cross-review --request-json FILE` reads one
  strict JSON `CrossReviewRequest` to append an exact reviewer node for an
  immutable candidate artifact.
- `GraphRead`: `gent orchestration read --conversation-id ID --graph-id ID`
  returns one scoped graph without provider-local run state.

Those commands reject malformed or unknown JSON fields, invalid typed bindings,
duplicate idempotency keys with changed payloads, cycles, self-dependencies, and
provider-native sessions or raw provider plans.
They persist/read graph intent only: no provider worker, scheduler, worktree,
or node attempt is created or implied.

Future extensions may add `DispatchReady`, terminal node transitions,
`GraphList`, and cursor-resumable `GraphSubscribe`. `DispatchReady` will remain
daemon-only and never be a client provider-spawn primitive.

## Fanout and isolation

Fanout creates graph intent only; dispatch creates immutable child runs and
node attempts transactionally. A node receives a daemon-leased isolated
worktree derived from a durable workspace/repository identity. It may not
reuse the parent working tree or another live node's lease. The daemon records
the exact base revision, worktree identity, lease owner/epoch, and approved
tool policy before a runner starts. Git mutation remains separately gated;
creating a worktree record or lease is not permission to execute Git.

Completion produces bounded normalized artifacts (patch digest, test summary,
finding set, terminal reason, and provenance), not opaque provider transcript
claims. Dependents unlock only from durable terminal facts that satisfy their
declared input contract. Failed, cancelled, lost, or unprovable nodes settle
explicitly and do not silently retry an external effect.

## Cross-vendor review

`CrossReview` binds reviewers to a candidate node's immutable artifact digest,
base revision, goal revision, policy revision, and worktree snapshot. Each
review produces structured `ReviewFinding` values: stable finding ID, severity,
category, location reference, evidence digest, disposition, and reviewer
attempt provenance. Provider output cannot select its own review disposition.

The reducer enforces a reviewer profile different from the candidate's provider
kind; a private Claurst reviewer additionally requires the private bridge
authority. A review cannot target its own node, mutable worktree, or a newer
candidate revision. Required-review policy can block merge/follow-on dispatch
until every required reviewer has terminally settled. Agreement is not assumed:
conflicting findings remain durable and require a typed user or policy decision.

## Lifecycle, goals, and reconnect

Node attempts use the existing conversation → run → turn lineage plus a graph
node/attempt identity. Provider events enter only through daemon-owned
normalized ingress, persist before broadcast, and drive the same bounded
activity states (generating, waiting for decision, waiting for subagents,
compacting, interrupted, terminal). Scheduler recovery rebuilds from durable
leases, node attempts, and cursor checkpoints; it drains process trees before
releasing a lease and never resumes an unverified native session.

Every dispatch resolves the currently active Gent goal revision immediately
before provider input. A stale, terminal, ambiguous, or mismatched goal blocks
or omits dispatch according to the pure goal reducer; provider adapters only
receive the validated projection. A graph transition similarly rechecks the
current parent, graph/node revision, host epoch, policy revision, receipt, and
goal revision in one ledger transaction.

Clients reconnect by negotiating, loading a graph snapshot, then subscribing
after its cursor. Cursor expiry, host epoch change, or a gap requires snapshot
reload; clients never infer a finished worker from missing output. Terminal and
native app render the identical graph and review frames.

## Storage and module boundaries

Add fresh `.gentd` schema tables only: graph/node/dependency/attempt, immutable
input/result artifact references, review request/finding/disposition, and
worktree lease bindings. There is no deployed migration or legacy behavior.
`gent-types` owns DTOs, `gent-core` owns pure graph/review reducers,
`gent-ports` owns ledger/profile/worktree/runner ports, `gent-store` implements
atomic SQLite operations, `gent-runtime` coordinates them, and only `gentd`
composes authority. `gent-cli` depends solely on protocol/types.

## Delivery order and gates

1. Define profile/catalog, graph, review, and artifact DTOs plus pure reducers;
   exhaustively test graph validity, terminality, retries, role/profile rules,
   review independence, and all stale/idempotency/policy/goal/epoch fences.
2. Add fresh SQLite ledgers and transactional graph reservation/settlement,
   worktree leases, snapshots/deltas, restart/recovery, and contention tests.
3. Add strict protocol frames and terminal graph clients; prove observer
   capability absence and malformed/bounded-frame rejection. The persistence
   foundation is complete; scheduler, snapshots, and dispatch remain future work.
4. Compose no runner yet. Add fake runner/profile/worktree integration tests for
   bounded scheduling, process-tree drain, backpressure, reconnect, and
   persist-before-broadcast.
5. After existing public-driver and private Claurst evidence gates pass, compose
   each approved profile one at a time with locked binaries/bridge, containment,
   live recordings, and cross-vendor review evidence. Custom profiles require
   their own signed catalog entry and evidence; no generic escape hatch exists.
6. Only after identical terminal IPC and native-client fixture coverage passes,
   authorize the separate native-app cutover and removal of direct app drivers.

This is planned work, not a claim of live multi-agent authority or app parity.
