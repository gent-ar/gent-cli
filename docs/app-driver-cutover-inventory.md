# App Driver Cutover Inventory

This is a development-only, public-semantics inventory for replacing the native
app's direct provider drivers with a Gent IPC client. It was compared read-only
against the app's Claude/Codex driver interfaces and public manifests on
2026-08-18. It is not provider evidence, a compatibility promise, or authority
to expose a capability.

Claurst is intentionally represented only by the generic lifecycle contract.
This document contains no Claurst endpoint, credential, binary location,
installation, routing, or private-bridge details.

The cutover checklist and remaining evidence gate are maintained in
[native-app cutover readiness](native-app-cutover-readiness.md).

## Cutover invariant

The app becomes presentation plus typed Gent IPC. It must not launch a
provider, parse provider stdout, retain provider-native session state, settle
provider decisions, or write Gent's ledger. Gent owns all of those concerns.
The current shipped daemon is an observer, so no row below is live merely
because a code path and deterministic test exist.

## Portable client contract

| App-facing behavior | Gent-owned portable meaning | Current code-level state | Required proof before app cutover |
| --- | --- | --- | --- |
| Create, prompt, queue, interrupt, follow-up | Receipt- and epoch-fenced conversation/run/turn commands; no text scraping | Typed protocol and durable intent state exist; live provider dispatch is withheld | End-to-end real-provider run with reconnect and exact-retry cases |
| Reconnect and cursor resume | Re-read bounded durable pages, then consume ordered facts after the last cursor | Generic event pages are cursor-only; activity follows the same durable-page contract | Live disconnect/restart evidence for every enabled provider |
| Model, effort, mode, provider choice | Persist the selected values on immutable run lineage before dispatch | Selection/switch lineage and provider runner inputs exist | Each provider's accepted values and rejection paths recorded |
| Cross-provider continuity | Start a child run with frozen Gent history; never transfer a native session | Fresh-context rendering and clear-context ordinal-zero guards exist | Codex → Claude → Claurst → Codex live transcript proving continuity |
| Native resume | Reuse a native session only for the same provider/run when the durable binding permits it | Claude and Codex runner/session seams exist | Resume/restart evidence for each provider and stale-binding rejection |
| Clear context | New child at ordinal zero, without a provider-native session | Guarded in runtime and Claude input path | Live assertion for all enabled adapters |
| Plan/review/start implementation | Gent alone stores a normalized trusted artifact and atomically approves/rejects a digest/revision | Reviewed-plan ledger/service exist and reject client plan injection | Provider-plan normalization plus approval and retry evidence |
| `/goal` and autonomous mode | Gent persists the goal and projects bounded context; provider text cannot grant authority | Goal projection is wired into Claude/Codex runner inputs | Goal lifecycle and permission-stop evidence for each enabled provider |
| Permissions | Gent correlates request, workspace, policy revision, receipt, and decision settlement | Typed policies/decisions and normalized ingress exist | Prompt and persistent-permission recordings; no uncorrelated allow path |
| Tools, thinking, usage, errors | Normalize bounded factual activity and diagnostics before client broadcast | Public normalizers and activity reducer exist | Strict fixture/live matrix for each advertised fact family |
| Child agents/tasks | Normalize parent/child IDs and terminal state without client inference | Codex and Claude public seams have child/activity handling | Live subagent lifecycle plus reconnect evidence |
| Attachments | Client stages bytes through Gent; provider input is only daemon-owned after capability proof | Durable local attachment staging exists; no live provider injection | Per-provider image/file capture and retry/resume proof |
| Automatic compaction | Normalize the failure, recover once from exact cursor into fresh frozen context | `tooFewGroups` recovery guards exist | Real compaction and recovery recording for each provider |

## Provider-specific public semantics to preserve

| Provider | App driver behavior that Gent must preserve | Deliberate Gent boundary |
| --- | --- | --- |
| Claude | stream-json prompt/result/tool/thinking flow; native resume and optional fork; model/effort; plan/auto/autonomous/bypass modes; rich provider-advertised permissions; queued input at tool-start; text/image input | Claude native identity stays in Gent. The app sees only normalized state and typed decisions. |
| Codex | app-server handshake/readiness; per-turn model/effort/policy settings; provider permissions that expire at turn end; live steer; queued input; model discovery; tool/plan/compaction and child-agent events; text/image model capability | Codex thread/turn and JSON-RPC request IDs stay in Gent. The app never speaks app-server. |
| Claurst | Generic typed start/resume/submit/decision/interrupt/event/terminal lifecycle, with durable cursor and normalized facts | Private bridge and all service configuration remain absent from public Gent and this inventory. |

## Concrete gaps found in this comparison

1. The app's direct drivers still cover their complete provider protocol and
   process lifecycle; Gent's Codex/Claude runners and private Claurst ingress
   are deliberately unadvertised, so terminal `gent` is not yet live promptable.
2. Model discovery, input attachment delivery, background/child transcript
   lifecycle, rich permissions, and compaction have code seams but not the full
   provider evidence matrix required for release.
3. Cross-provider frozen history has deterministic tests, but no complete live
   Claude/Codex/Claurst switching recording. Native session IDs must never be
   reused across that boundary.
4. The native app must remove its direct drivers only after Gent advertises the
   negotiated capabilities and the proof column above is complete. There is no
   fallback or dual-run migration path.

## Verification anchors

- Gent protocol ownership: [Flutter handoff](flutter-handoff-v1.md).
- Live/evidence gate inventory: [implementation status](implementation-status.md).
- Public provider fixture provenance: `fixtures/public-driver-transcripts/` and
  opt-in `drivers_transcript/`; do not fabricate missing recordings.
- Contract checks: `cargo test --workspace --all-features` and
  `python3 tools/check-architecture.py`.
