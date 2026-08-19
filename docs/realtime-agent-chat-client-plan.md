# Realtime agent-chat client contract

This is the missing end-to-end delivery contract for making Gent the realtime
agent-chat backend of both the `gent` terminal UI and the Gent native app. It
does not authorize live providers in the current observer daemon.

The app-driver retirement criteria and explicit remaining authority/evidence
gates are collected in [native-app cutover readiness](native-app-cutover-readiness.md).

## Ownership

Providers are the source of raw external facts. `gentd` is the sole source of
truth exposed to clients: it validates, normalizes, durably orders, and streams
those facts. The terminal and native app are equal Gent clients. Neither starts
a provider, parses provider stdout, writes the ledger, or derives lifecycle
state from timers or message text.

```text
provider process / private Claurst bridge
  -> daemon-owned driver and session -> normalized durable event ledger
  -> cursor-resumable local IPC subscription -> terminal and native app
```

Raw provider payloads, credentials, endpoints, and provider-native session IDs
do not cross the public client contract. Gent may keep a provider-native resume
token only in the daemon-owned execution edge and only for the matching locked
provider/runtime; it never crosses a provider switch.

## Target realtime connection lifecycle

One Gent profile has one `gentd` host and one private `.gentd` data directory.
Opening an agent-chat tab starts or locates the pair once, then the native app
keeps a long-lived local IPC connection. `gent` terminal UI does the same. A
command invocation may bootstrap the host, but it is never the per-prompt
transport. This is the target authority composition, not a claim that the
current observer host can start a provider.

The host is demand-started, not a permanent background service. It stays alive
while it owns active work, a pending decision, or connected clients, and may
exit after an explicit idle policy. All conversations share that one host and
its one ledger. The app must multiplex them over IPC; it must never start one
`gent` or one provider process per conversation.

Only the daemon scheduler starts provider processes. It applies one
daemon-wide resource budget plus provider-specific limits, keeps excess work in
the durable prompt queue, and drains owned process trees before stopping. Those
limits and the queue are daemon state, not app counters or terminal state.

Each conversation/run must also retain its canonical workspace binding in the
Gent ledger. A multi-conversation profile may contain several workspaces, so a
daemon cwd, app cwd, or client-local workspace cache is not authoritative. The
daemon validates that binding before launch and passes it only to the matching
provider process; a prompt never supplies a process working directory.

1. Bootstrap with `gent --data-dir <profile> status` only if no host exists.
2. Connect, send `hello`, and require `negotiated` before every extension.
3. Load conversation index and the selected conversation's bounded read pages.
4. Attach one cursor-resumable event/activity subscription for the visible
   conversation/run. Render only durable pages or ordered deltas.
5. On disconnect, epoch change, cursor expiry, or resync request: invalidate
   the affected projection, reconnect, negotiate, reload bounded durable pages,
   then attach after the last acknowledged cursor. Do not synthesize missed states.

Snapshots, recovery caches, mirrored state, and state replacement are prohibited as client or
daemon mechanisms. Gent rebuilds by reading bounded durable pages and replaying normalized facts
after a cursor; it never retains provider-native session state as a recovery artifact. A derived
in-memory view is optional and disposable, never serialized or sent as authoritative state; on an
epoch/cursor mismatch it is discarded and rebuilt from immutable facts (from cursor zero when
necessary).

The client may maintain display-only local view state such as scroll position,
selected row, and input text. It must discard any assumption about a run when
the host epoch changes.

## Terminal and native-app parity

Running `gent` opens a first-class interactive agent-chat client, not a
diagnostic shell. It has the same Gent-owned conversations, sessions, history,
composer, provider/model/effort/mode selectors, plan review, `Start
implementing`, clear context, permissions, login choices, live activity, tools,
and reconnect behavior as the native app. A terminal-specific layout or
keyboard interaction must never change the underlying command or reducer.

The native app adds private product surfaces around that shared harness: local,
LAN, and relay presentation; IDE/editor integration; system UI; device pairing;
speech/voice; and app-specific automations. Those surfaces consume Gent state
and submit Gent commands. They do not fork the agent runtime or turn Flutter
into an alternate provider host.

Feature availability is capability-negotiated, so the terminal and native app
both hide or disable a Gent feature until its daemon authority exists. The
native app may offer additional app-only controls, but it cannot make Claude,
Codex, Claurst, MCP, Git, or a plan lifecycle work when Gent has not advertised
the matching capability.

## User journeys

### Browse and resume

When the tab opens, Gent returns conversation identities, titles, current
selection, run lineage, and the latest bounded durable activity page. Selecting a
conversation loads its ordered normalized history and attaches its live stream.
The terminal browser uses the same requests. Conversation rows, session cards,
thinking/loading indicators, waiting-for-command, and waiting-for-subagents are
projections of daemon facts, never app-owned booleans.

### Create and prompt

Creating a conversation submits a typed selection of provider, model, effort,
and mode. Gent durably creates the conversation/root run and returns a receipt.
Sending a prompt submits a receipt/idempotency-bound command to that run. The
daemon persists the accepted intent before it can launch any provider work. Its
accepted response identifies that exact durable conversation, run, and turn;
clients use those identities to follow work and never infer them from state.
For a new direct `gent <prompt>` conversation, the CLI selection defaults to
Ask unless the user explicitly selects another mode.

`/goal <summary>` is a separate, typed Gent command bound to an existing
conversation/run. Gent validates and durably revisions the active goal, then
projects that exact revision into each provider adapter as controlled context.
Claude, Codex, and the credential-free private Claurst bridge may consume that
projection, but cannot create, rewrite, settle, or retain it as provider state.
Autonomous is a permission policy, not a goal or authority bypass: it remains
subject to the same epoch, sandbox, binary-lock, decision, and terminal-settle
requirements.

The daemon then validates authority, current epoch, permission policy,
compatibility, sandbox, and locked binary identity. Only after those gates pass
does it start or reuse its provider session. Every normalized provider fact is
persisted with a monotonically ordered cursor before it is emitted to clients.
A provider failure, cancellation, refusal, or lost session similarly reaches a
terminal durable state; a spinner never depends on a process-local callback.

If the selected public provider CLI is missing, the prompt path may make one
daemon-owned, receipt-backed provisioning attempt before the provider launch.
The native app supplies a Node runtime location with its installed Gent pair;
it does not bundle, launch, update, or retain Claude Code/Codex itself. Gent
uses that runtime's `npm` to pack an exact policy-selected package without
lifecycle scripts, verifies the tarball SHA-512 SRI, then installs that verified
archive into a private Gent prefix before discovering and locking its executable.
A retry never repeats an ambiguous install effect.

### Follow-ups and session continuity

The active run remains daemon-owned. A follow-up is another typed prompt bound
to that run; the native app does not invoke `claude`, `codex`, or a bridge. Gent
may use the matching provider-native resume mechanism after it rechecks the
run-version lock. If it cannot safely resume, it constructs provider-neutral
context from the frozen durable history boundary instead of silently attaching
an unrelated provider session.

### Provider/model change, plan approval, and clear context

Changing selection or approving `Start implementing` creates an immutable child
run. The approval records the exact plan revision/digest, policy revision,
receipt, host epoch, provider/model/effort/mode, and context policy. Preserve
uses the recorded normalized-history ordinal. Clear context uses ordinal zero,
the approved plan handoff, and no provider-native resume token. Durable history
and lineage remain visible in both clients.

### Permissions and login

A provider request becomes a typed, durable Gent decision. The client renders
it and returns a typed response; approved Plan-mode rules remain Plan mode and
existing scoped grants prevent repeated equivalent prompts. Login is a typed
`askTool` challenge with account/API-key route selection; credentials stay at
the provider-owned execution edge. The terminal and native app render the same
challenge and never transport credential text through public IPC.

## Required authority work

Before this contract is advertised, implement and prove all of the following:

- A daemon-owned public Claude/Codex session runner that retains bounded stdout
  and stderr, normalizes each documented frame, persists it before publication,
  manages backpressure, and terminates/recovers process trees safely. Its
  persist transaction includes source identity, cursor, session binding,
  projection delta, and terminal settlement; restart replays durable cursors
  before it returns new deltas. See [the atomic session/restart matrix](native-app-cutover-readiness.md#atomic-session-and-restart-proof).
- Private app-owned Claurst bridge ingress with the same normalized lifecycle
  contract. Public Gent contains only the bridge port, never endpoints or
  credentials.
- Bounded durable read pages plus cursor-resumable live event/activity subscriptions for
  conversation browsing, transcript, runs, turns, tools, decisions, and plans.
- Durable reviewed-plan storage and approval reservation, including policy/epoch
  fences, idempotency, parent/run lineage, clear-context ordinal zero, and
  terminal settlement.
- Goal-ledger lookup at every provider turn, with only the currently active
  revision projected into Claude, Codex, or the private Claurst bridge. A goal
  transition cannot be silently reused as stale provider context.
- Live composed capabilities that advertise each feature only when its concrete
  handler, authority profile, binary lock, sandbox, fixtures, and evidence are
  present. Observer mode must hard-disable every live provider path.
- Native-client IPC fixtures for bootstrap, reconnect/resync, create, prompt,
  follow-up, selection child runs, review/start/reject, permissions, login, and
  provider terminal outcomes.
- Redacted real-provider evidence for every required Claude/Codex cell and
  private Claurst bridge CI evidence.
- Signed provider-package policy that binds each permitted `npm` package,
  version, integrity, Node-runtime compatibility range, private install prefix,
  receipt recovery, and locked-binary verification before prompt-triggered
  provisioning can be advertised.

## Acceptance scenario

An agent-chat user can open the native tab or run `gent`, browse conversations,
select one, read durable pages, submit a new prompt or follow-up, and watch
the same ordered normalized state arrive in both clients. If either client
disconnects, it reconnects from a cursor without duplicate output or invented
loading. Model/provider changes and clear context produce auditable child runs.
No client launches, resumes, or interprets a provider directly.
