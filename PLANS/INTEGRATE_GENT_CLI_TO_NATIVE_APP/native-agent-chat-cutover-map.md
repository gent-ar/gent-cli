# Native Agent Chat cutover map

Paths are relative to `/Users/ivanmatiasfort/Clouseau/clouseau-app` unless prefixed `crates/`, which
is `/Users/ivanmatiasfort/Clouseau/gent-cli`.

## Decision

`gentd` is the sole agent-chat authority. The native app packages and connects to it over local IPC;
it never starts the `gent` terminal program. `gent` and Flutter are two presentations over one
authority and one data directory. The full rule is in `gentd-source-of-truth-contract.md`.

Flutter owns presentation-local state only: selected rail, modal visibility, draft text, focus,
scroll, keyboard, canvas. Gentd owns every durable chat fact and every provider-facing operation.

The existing bridge proves packaging and local IPC work. It is not the target: it mirrors Gentd
transcript and activity into Flutter-owned `AgentChatState`, sends a reconstructed history envelope
when converting a native chat, and reads several unrelated resources to rebuild one screen. It is
replaced in one cutover, not retained beside the new path.

## Foundations to keep

| Native code | Keep for | Change |
| --- | --- | --- |
| `app/lib/service/gentd/gentd_app_runtime.dart` | Bundled daemon lifetime, data directory, MCP config handoff | Resolve the directory via `gentd --print-data-dir`; make startup connect-first; publish connection state. UI reducers never own process readiness. |
| `app/lib/service/gentd/frb_gentd_ipc_transport.dart`, `app/lib/rust/api/gentd_ipc.dart` | Platform-neutral byte transport | Keep as transport only |
| `app/lib/service/gentd/gentd_ipc_client.dart` | Framing, handshake, attachment staging, existing intents | Add the projection client; drop the six read capabilities it replaces |
| `app/lib/widget/agent_chat/*` | Mature visual components | Feed them Gentd view models; remove dependence on adapter processes and reconstructed provider events |
| `app/lib/widget/agent_chat/sidebar/*` | Sidebar presentation | Replace provider- and cache-derived rows with projections |
| `app/lib/widget/agent_chat/canvas_panel.dart` | Native canvas | Keep native-only, outside the chat contract |

## Code deleted at cutover

| Native code | Why it cannot remain |
| --- | --- |
| `gentdHistoryEnvelope` (`app/lib/provider/agent_chat/agent_chat_gentd.dart:10`) | Feeds a provider a synthesized `<conversation_history>` text blob instead of the authoritative conversation. Loses typed timeline structure and makes history depend on the Flutter cache. |
| Transcript-to-`ChatMessage` reconstruction (`agent_chat_gentd.dart:616-626`, `:685`) | The daemon owns ordered transcript and lifecycle facts. A second reducer drops identities and creates race paths. |
| `selectedModel`-keyed download filter (`agent_chat_gentd.dart:487`, `:501`) | Both sites drop events when `state.selectedModel != modelId`. A download must be correlated by daemon-issued request ID; two prompts may legitimately await the same model, and switching models mid-download silently orphans the indicator. |
| Out-of-band permission refetch (`agent_chat_gentd.dart:99`, `:173-189`, `:420`) | Pending decisions are fetched by a separate `pendingPermission()` call fired on attach and on every `toolActivity` transcript event, into the `_gentdPendingPermissions` map. That request races the stream it is triggered by. Decisions must arrive in the ordered projection beside the tool and turn that own them. |
| `agent_chat_adapters.dart`, `agent_chat_turn.dart`, `agent_chat_send_dispatch.dart`, `agent_chat_stale_processing.dart` for Gentd conversations | Flutter must not launch Claude, Codex, Claurst, llama.cpp or MCP servers, and must not resume a provider. Those belong to Gentd for every provider. |
| Flutter-persisted provider/model/effort/mode, transcript, activity, checkpoints and run identity for Gentd conversations | Durable daemon facts. A second store makes reconnect and cross-client use incorrect. |

Deletion is part of the cutover. No permanent Gentd branch beside an older native authority.

## Semantic changes in the native UI

### Mode and permissions

The native app treats these as one setting — `default`, `plan`, `autonomous`, `auto-accept edits`,
`bypass permissions` — in `app/lib/widget/agent_chat/input_chips_row.dart`,
`app/lib/service/agent_chat/model_settings_controller.dart`, `app/lib/model/agent_adapter.dart` and
the Claude/Codex driver settings. That prevents choosing an approval policy independently of work
style.

Gentd already separates them. `AgentChatSelection`
(`crates/gent-types/src/agent_chat.rs`) carries provider, model, effort and
`AgentChatMode` = `Ask | Plan | Agent`. Permission policy is a separate revisioned `PolicyRecord`
(`crates/gent-types/src/policies.rs`) written through `permission-policy-v1`. The native work is to
render both as two independent controls from catalog records — not to invent the split.

| Control | Source | Meaning |
| --- | --- | --- |
| Mode | `mode` catalog; initial entries `Ask`, `Plan`, `Agent` | What kind of work the agent performs |
| Permissions | `permission-policy` catalog over `PolicyRecord` | The approval policy, independent of mode. Gentd maps a policy to provider capability and reports unsupported choices rather than silently widening authority. |

Flutter hard-codes neither the entries nor their order; both come from the record's `id` and
`ordering`. `Bypass` is an explicit, separately confirmed dangerous choice, never a mode and never
the default. The app must not convert a mode selection into an approval decision, or change policy
when the user switches provider.

Contract after item 2:

```text
selection        = provider + model + effort + mode
permissionPolicy = separately revisioned workspace or conversation PolicyRecord
switchSelection(conversation, parentRun, selection, contextPolicy)
updatePermissionPolicy(scope, expectedRevision, policy)
```

Every switch produces an immutable child run with preserved context. The selector updates only after
its receipt returns. Policy updates need their own receipt and revision-conflict behavior. This
removes the current class of UI resets where a later local refresh overwrites the selected model or
mode.

### Composer controls

| Surface today | Gentd-backed replacement |
| --- | --- |
| Provider/model picker | `provider` and `model` catalogs; each entry exposes readiness and supported selections. Ordering comes from the record's `ordering` field — Gentd's default catalog places Claurst first, and Flutter never encodes that. |
| Effort picker | `effort` catalog filtered by provider/model capability, with the persisted current value shown |
| Mode chip | `mode` catalog |
| Permissions chip | `permission-policy` catalog: current policy, category detail, confirmation requirement, and an explanation when a provider cannot express a policy |
| Tools/MCP chip | `tool-source` catalog with enabled state, health and per-run tool activity; Flutter never writes provider config |
| Attach | Stage through `attachments-v1`, retain the receipt, render only daemon-accepted state |
| Git/worktree chip | Workspace projection: branch, worktree, dirty/add/delete counts, safe native open actions |
| Checkpoint/fork/resume | `agent-chat-intents-v1` records and intents |
| Draw | Native-only; the resulting attachment is staged like any file |

### Header, timeline, activity and process surfaces

| Native surface | Current derivation | Target fact |
| --- | --- | --- |
| Files chip | Flutter messages/attachments | Conversation attachment projection |
| Context chip | Flutter token estimate | `ConversationActivityFact::ContextUsage`, which already carries `used_tokens` and `window_tokens` |
| Processes chip/modal | `ToolUseInfo` reconstructed in Flutter | Work-item facts: stable command ID, phase, output availability, cancel intent |
| Timeline chip and inline timeline | Ordering inferred from `ChatMessage` | Cursor-ordered transcript plus lifecycle facts |
| Tasks/plan chip | Adapter-specific tool parsing | `reviewed-plan-v1` |
| Subagents chip/modal | Child records reconstructed from tools | `ConversationActivityFact::SubagentStarted` already carries `child_id` and `parent_tool_use_id`; the work-item read supplies task text, model and output |
| Errors and permission badges | Local status and pending-card state | Conversation attention projection |
| Thinking/loading indicator | Flutter streaming fields | Ordered activity phase: preparing provider, downloading model, authenticating, thinking, invoking tool, waiting for command, waiting for subagent, awaiting permission, interrupted, terminal |

The raw-reasoning setting stays client-local and controls visibility of Gentd's persisted thinking
events only. It never decides whether the daemon records them.

The working indicator renders exact daemon facts — `Downloading Qwen 3 1.7B · 54%` with a cancel
action, `Waiting for command`, `Waiting for 2 subagents` — never speculative readiness wording.

### Sidebar and navigation

| Rail | Native behavior to retain | Projection/intents needed |
| --- | --- | --- |
| Conversations | Title, recap, preview, timestamps, status, search, create/open/fork/resume | Sorted summaries with a search cursor and durable actions |
| Sessions | Workspace grouping, active conversation, reopen, ordering | Session summaries plus select/open/create intents |
| Prompts | List, search, save, rename, insert, delete | `prompt-templates-v1` records and mutations |
| Docs | Grouped workspace discovery, open and attach | `workspace-documents-v1` references |
| Activity | Attention list, running processes, stop | Cross-conversation lifecycle/attention feed scoped to workspace and session |
| Automations | Definitions, run history, enable/edit/delete/run | `automations-v1` catalog and run history plus intents |

Search, selection and scroll stay local. Row data and all mutations become Gentd-owned.

## Projection contract to add first

`agent-chat-projection-v1` does not exist in code today. It replaces and deletes six granular
capabilities that `gentd_ipc_client.dart:19-28` currently negotiates —
`conversation-index-v1`, `agent-chat-conversations-v1`, `agent-chat-transcript-v1`,
`agent-chat-turn-follow-v1`, `conversation-activity-v1`, `agent-chat-sessions-v1`. The mutation
capabilities (`agent-chat-intents-v1`, `agent-chat-permissions-v1`, `attachments-v1`,
`local-models-v1`, `permission-policy-v1`) survive unchanged.

```text
readWorkspace(workspace, cursors)      -> workspace snapshot
readConversation(conversation, cursors)-> conversation snapshot
followConversation(conversation, cursors) -> ordered deltas
followWorkspace(workspace, cursor)     -> ordered sidebar/activity deltas
readWorkItem(id, page)                 -> bounded tool/command/child content
intent(receipt, operation, payload)    -> receipt or typed failure
```

Conversation snapshot fields:

- Conversation and run IDs, title, recap, preview, timestamps, attention state
- Current selection, available mode/permission/effort catalogs, provider readiness, prompt-admission
  state
- Typed transcript, thinking events, activity timeline, tool output references, commands, subagents,
  tasks/plan, pending decisions, attachments, checkpoints
- Workspace, MCP connector and Git/worktree projections
- Independent cursors for transcript, activity and workspace streams

The daemon validates source identity and cursor monotonicity. Every mutation carries an idempotency
receipt. A reconnect obtains a snapshot then follows from the returned cursors; it never rebuilds a
conversation from local cache or retries a provider command itself.

## Native implementation sequence

1. Define the projection DTOs and intent receipts in Gentd; expose them through
   `gentd_ipc_client.dart`. Add client types only — no UI change.
2. Make `GentdAppRuntime` a connect-first resident-daemon owner that publishes connection state.
   Leave the FRB byte transport unchanged.
3. Introduce one `GentdAgentChatController` that reduces snapshot plus deltas into view models. It
   replaces `app/lib/provider/agent_chat/agent_chat_gentd.dart` — one class, not a new layer beside
   it — and contains no provider process, protocol parser, transcript persistence or permission
   engine.
4. Wire the conversation rail, message timeline, thinking/activity state, command and subagent views
   and permission cards to that controller. Verify reconnect by restarting the UI mid-turn.
5. Replace the composer with catalog records and the selection/policy intents, including the
   independent Permissions chip. Delete the old autonomy vocabulary and all adapter-owned
   mode/permission transitions in the same change.
6. Wire sessions, prompts, docs, activity, MCP, Git and automations to their projections. Canvas
   stays native-only.
7. Remove the Gentd mirror and native provider authority paths. A conversation is then either a
   Gentd conversation or absent from Agent Chat; there is no dual writer.

## User-flow acceptance

- Open the same conversation in `gent` and the app: identical title, selection, transcript, timeline
  and active work.
- Start with the default local model absent: one durable prompt, a real download percentage, working
  cancellation, then exactly one execution after download.
- Switch model, provider, effort, mode and permissions mid-conversation: the new immutable run has
  full context and the UI does not revert the selection.
- Exercise each permission policy under Ask, Plan and Agent, including a provider that cannot express
  a requested policy — that case must show the record's `unavailable_reason`.
- Attach files, use MCP, run Git/worktree actions, inspect tool output, stop a process, watch a
  subagent, resolve a permission — each from either client.
- Fork, resume and restore a checkpoint from either client; restart the app mid-run and reconnect
  with no duplicated messages or commands.
- Confirm sidebar activity, title/recap, prompt library, docs and automations reflect the other
  client's changes.

## What this does not require

Flutter does not duplicate Claude Code, Codex or Claurst protocol behavior, provider installs, login,
MCP transport, llama.cpp process control, tool parsing, transcript repair or session resumption.
Gentd centralizes all of it. Flutter needs the projection and a small set of typed intents.
