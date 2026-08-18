# Native-app cutover readiness

Gent is the single agent harness and source of truth. `gent` and the native
app are equal clients of one long-lived local `gentd` IPC connection. The app
may add local/LAN/relay presentation, IDE, system UI, voice, pairing, and
app-only automations, but it must never launch a provider, parse provider
stdout, retain provider-native session state, own provider lifecycle logic, or
write Gent's ledger.

This is a clean, zero-user cutover. There is no data migration, legacy mode,
compatibility bridge, direct-provider fallback, or dual-run period. Gent owns
the fresh `.gentd` data directory; the app retains only display-local state.

## Required end state

The terminal and native app must use the same negotiated capability, request,
snapshot, delta, receipt, epoch, reconnect, and cursor-resume contracts for
conversations, prompts/follow-ups, browsing, model/effort/mode/provider
selection, permissions/login, tools, reviewed plans, `/goal`, and clear
context. Neither client may infer lifecycle state from text, timers, or native
provider sessions.

A provider/model change creates an immutable child run. Preserve context uses
Gent's frozen normalized-history ordinal; it never transfers a Claude, Codex,
or Claurst native session across a provider boundary. Clear context creates a
child at ordinal zero with no provider-native session. Automatic compaction
must normalize a provider-recognized compaction condition, settle/drain the old
work, and recover once into a fenced fresh child from the frozen history; a
client never silently starts an unrelated conversation or invents a summary.

`/goal` is a Gent-owned, revisioned goal projected as bounded context into all
adapters. Autonomous is a Gent permission policy, never a goal or authority
bypass. Provider plans are normalized by Gent; clients may review/approve a
plan but can never inject one.

## App update and dependency boundary

The installed Gent pair updates through its signed, independent updater. A
provider or harness fix therefore ships in Gent and does not require an app
provider-driver release. The app may bundle Node only. On a consented prompt
for a missing Claude Code or Codex CLI, a future approved Gent authority may
use that app-supplied, identity-locked Node/npm pair for one receipt-backed,
signed-policy package installation into `.gentd/providers/npm-global`. Gent
then re-discovers and locks the executable. The app never bundles, installs,
updates, or falls back to either CLI. Claurst has no public npm path; its
credentials, endpoints, routing, and bridge evidence remain private.

## Cutover gate

Do not remove app drivers or advertise live provider capabilities until every
condition below is independently proven:

1. A daemon-owned Claude/Codex bounded lifecycle host and the private Claurst
   bridge normalize durable facts before broadcast, apply backpressure, drain
   process trees, settle terminals, and serve snapshots/deltas/reconnect.
2. The approved composition rechecks an enforced sandbox attestation, signed
   compatibility/package policy, exact locked binary, durable receipt,
   idempotency key, host epoch, parent/run revision, and permission policy.
3. The strict evidence matrix is complete: Claude persistent permission,
   compaction, and malformed tolerance; Codex malformed tolerance; and private
   Claurst bridge/CI evidence. Existing recordings are not substitutes for a
   missing strict cell and must never be fabricated.
4. The terminal and app parity matrix covers create/prompt/follow-up, provider
   switching, preserve/clear context, plan review, `/goal`, permission/login,
   attachments, subagents/tasks, compaction, terminal settlement, and
   disconnect/restart cursor recovery for every advertised provider.
5. Required workspace, architecture, installer/update, IPC, and coverage gates
   pass against the released composition. Observer mode has no provider
   capability, executable inspection, login, provisioning, or launch route.

Only after these gates and negotiated capabilities are present may the app make
one atomic source cutover: move each surface to Gent IPC, then delete its
corresponding direct provider drivers, launchers, stdout parsers, session
stores, lifecycle reducers, authentication paths, and tests. CI must prove the
app has no provider process launch, raw provider parsing, or Gent-ledger write.
A disconnected Gent host is rendered as unavailable/reconnecting, never as a
reason to revive an app driver.

See [the detailed inventory](app-driver-cutover-inventory.md), [realtime client
contract](realtime-agent-chat-client-plan.md), and [authority/evidence
status](implementation-status.md).
