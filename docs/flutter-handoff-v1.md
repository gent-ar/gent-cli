# Flutter Consumer Handoff v1

This is the versioned, language-neutral handoff contract for a future Flutter
agent-chat integration. It is intentionally a client contract, not Dart code,
a provider contract, or permission to enable daemon authority.

## Scope and compatibility

Flutter must use the installed `gent`/`gentd` pair, never launch Claude,
Codex, Claurst, MCP, or Git directly. It may run `gent --data-dir <private-dir>
status` to start the local host, then keep its own long-lived local-protocol
connection to the same private data directory. A command invocation is only a
host bootstrap or terminal fallback; it is not one process per prompt.

The version-1 codec source of truth is
[`fixtures/ipc-contract/manifest.json`](../fixtures/ipc-contract/manifest.json).
It specifies canonical JSON and exact `u32` big-endian length-prefixed UTF-8
frames. CI validates it with:

```sh
cargo run -p gent-testkit --bin validate-ipc-fixtures -- fixtures/ipc-contract/manifest.json
```

Flutter must ship a codec test against the fixture before enabling this path.
It must reject frames over 16 MiB, malformed/incomplete JSON, unexpected frame
types, and any protocol range with no overlap. The current range is `1..=1`;
do not assume a higher version is backward compatible.

On macOS/Linux the endpoint is `<private-dir>/gentd.sock`. On Windows it is a
named pipe derived from the selected data directory; Flutter should use the
installed protocol client/adapter for that platform rather than invent a pipe
name. The data directory must be shared by every Gent invocation for one app
profile and must remain private to that OS user.

## Connection sequence

1. Start or locate the pair with `gent --data-dir <private-dir> status`. Pass
   `--no-autostart` only when host launch is deliberately forbidden.
2. Connect to the local endpoint and send exactly one `hello` frame before any
   other frame. Offer only capabilities the Flutter client implements.
3. Require `negotiated`; verify its protocol is supported and use only the
   returned capability intersection. A missing capability is unavailable, not
   an invitation to retry with a different frame.
4. Issue the selected capability's typed request over that same connection.
   Additive endpoints use their own frame enum after negotiation; they are not
   generic `command` frames.
5. On disconnect, protocol error, `cursorExpired`, or a subscription
   `resyncRequired`, reconnect, negotiate again, replace local state from the
   relevant snapshot/page, then resume from the returned cursor.

Never scrape human-oriented terminal output as the app protocol. `gent` is a
convenient bootstrap and manual UI; local IPC plus the fixture is the durable
app boundary.

The required browse/create/prompt/follow-up/reconnect behavior is defined in
[the realtime agent-chat client contract](realtime-agent-chat-client-plan.md).

## Current readable state

| Need | Capability and frame family | Current authority |
| --- | --- | --- |
| Host protocol/epoch | base `statusRequest` → `status` | Available in observer mode |
| Durable event resume | `event-stream-v1` | Available; cursor ordered |
| Conversation identities | `conversation-index-v1` | Available; content-free |
| Run/turn status and lineage | `conversation-status-v1`, `conversation-timeline-v1` | Available; no provider session IDs |
| User prompt pages | `conversation-content-v1` | Unix only; protected, bounded pages |
| Update attempt status | `runtime-maintenance-v1` | Only in its explicit read authority profile |
| Signed cached update discovery | `runtime-update-check-v1` | Only in its explicit metadata authority profile |
| Conversation activity | `conversation-activity-v1` | Reserved; observer does not advertise it |
| Chat summaries/transcripts/intents | `agent-chat-*-v1` | Reserved except isolated persistence testing |

Use `ConversationActivity` only when the negotiated activity capability exists.
Its `schemaVersion`, conversation ID, run ID, host epoch, cursor, revision, and
activity sequence are all mandatory consistency boundaries. Render
`thinking`, `waitingForCommand`, and `waitingForSubagents` only from a valid
authoritative snapshot/delta; do not infer them from text, timers, or a prompt
receipt. No current Flutter integration may present these values as live
provider truth because provider fact ingress is not enabled.

`gent update auto status` is a signed external-helper command, not IPC and not
a daemon scheduler. `runtime-maintenance-v1` reports a durable attempt only
when its separate capability is negotiated; it cannot fetch or activate an
update. The installed Gent pair enables its platform scheduler by default; a
future native app relies on that pair rather than implementing a second updater.

## One writer and epoch rule

`gentd` is the only future ledger writer. Flutter is a protocol client: it
must not open the SQLite ledger, write its files, spawn a provider, or maintain
a competing lifecycle projection. Persist the last accepted cursor and relevant
host epoch with the app's local view, but treat a newly observed epoch as a
state boundary: discard in-flight assumptions, reload snapshots/pages, and
re-negotiate capabilities.

For a base `command`, send the currently observed `hostEpoch`, a new receipt
ID, and a stable idempotency key. Retry only the exact same command with those
same values. A receipt/event with another epoch requires resynchronization;
never silently retarget it. Future agent-chat intent retries similarly retain
their original request and receipt identities. Flutter must never manufacture a
provider acknowledgement or decision settlement.

The app must ensure one local Gent profile/data directory has one active host
writer. Starting `gent` concurrently is safe only because `gentd` owns the
host lock and epoch. Flutter must not bypass that lock with its own daemon,
alternate database writer, copied socket, or direct provider process.

## Explicitly unavailable today

The shipped observer daemon deliberately routes no live Claude, Codex,
Claurst, MCP, or Git work. It does not advertise authoritative activity,
transcript streaming, live agent-chat intent authority, or a private bridge.
`gentd --agent-chat-authority` is an isolated persistence test profile: its
create/send/queue receipts mean `awaitingProvider` or `queued`, never a
provider launch or response.

Reviewed-plan approval, `Start implementing`, model/effort/mode selection at
approval, and `Clear context and proceed` are planned typed Gent commands, not
current Flutter controls. Their required child-run and context semantics are in
[the reviewed-plan execution contract](agent-chat-execution-plan.md). Flutter
must render Gent's state and submit its choices only; it must not duplicate the
plan, permission, context, provider-selection, or lifecycle logic.

Claurst remains an app-private bridge implementation and private-CI concern;
no credential, endpoint, or routing configuration belongs in public Gent or
the Flutter-facing protocol. Device pairing and application-specific UI
automations remain Flutter-owned. `gent-canvas`, `gent-forge`, agent
automations, live MCP/Git authority, and seamless live provider switching are
follow-on runtime work, not capabilities a client may enable today.

Before Flutter enables provider-backed chat, the standalone repository must
first complete the remaining real-provider evidence, private Claurst evidence,
and an approved authority composition. The current gate inventory is
[implementation status](implementation-status.md); it is authoritative over
this guide if a capability is not actually negotiated.
