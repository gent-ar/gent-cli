# Agent Chat parity inventory

Surface-by-surface detail behind backlog items 3 and 5. `native-surface-disposition.md` decides
ownership; this doc records what each surface needs from Gentd. Where the two disagree, the
disposition map wins.

Paths are relative to `/Users/ivanmatiasfort/Clouseau/clouseau-app` unless prefixed `crates/`, which
is `/Users/ivanmatiasfort/Clouseau/gent-cli`.

## Target ownership

Gentd owns durable conversation identity, runs, turns, normalized transcript, activity, permissions,
selection, attachments, provider lifecycle, model download, MCP execution configuration and workspace
projections. `gent` and the native app are equal reactive IPC clients. Neither launches a provider or
treats a local cache as authority.

## Inventory

The "Gentd today" column names the declared capability where one exists
(`crates/gent-runtime/src/catalog.rs`). Most domains already have one; the work in item 5 is
completing their fields, not creating them.

| Surface | Native source | Gentd today | Required addition |
| --- | --- | --- | --- |
| Chat shell | `app/lib/pages/tabs/agent_chat_tab.dart`, `app/lib/widget/agent_chat_panel.dart` | n/a | Client shell only |
| Conversations | `app/lib/provider/conversation_list_provider.dart`, `app/lib/widget/agent_chat_conversation_sidebar.dart` | `conversation-index-v1`, `agent-chat-conversations-v1`, `agent-chat-transcript-v1`, `conversation-status-v1` | Paged summaries with title, recap, preview, workspace, timestamp, count, current selection, lifecycle and attention |
| Sessions | `app/lib/widget/agent_chat/sidebar/sidebar_conversations_sessions.dart`, `app/lib/provider/session_index_provider.dart` | `agent-chat-sessions-v1` | Workspace, title, active conversation, ordered conversation IDs, open state, created/opened timestamps, reopen and select intents |
| Prompt templates | `app/lib/widget/agent_chat/sidebar/sidebar_prompts_knowledge.dart`, `app/lib/provider/prompt_library_provider.dart` | `prompt-templates-v1` | List, search, create, rename, delete, insert-ready body |
| Docs and knowledge | `app/lib/widget/agent_chat/sidebar/sidebar_prompts_knowledge.dart`, `app/lib/util/knowledge_sources.dart` | `workspace-documents-v1` | Discovery grouped as Project, `.gent`, Docs and Global, with stable file identity and attach/open actions |
| Activity | `app/lib/widget/agent_chat/sidebar/sidebar_activity.dart` | `conversation-activity-v1` | Cross-conversation feed of active command, tool, subagent, permission, question and terminal facts; interrupt stays an intent |
| Automations | `app/lib/widget/agent_chat/sidebar/sidebar_automations.dart`, `app/lib/provider/automation_provider.dart` | `automations-v1` | Definition list, selected run history, enablement, manual run, edit, delete; scheduler lifecycle later |
| Messages | `app/lib/widget/agent_chat/message_list.dart`, `message_bubble.dart`, `streaming_bubble.dart` | `agent-chat-transcript-v1`, `agent-chat-turn-follow-v1` | Typed message/timeline reducer fed by the projection |
| Tool timeline | `app/lib/widget/agent_chat/tool_timeline.dart`, `tool_presentation.dart` | `conversation-activity-v1` gives identity and phase only | Work-item content: tool input, output body, diff, content blocks, child linkage |
| Commands and processes | `app/lib/widget/agent_chat/process_output_modal.dart` | `ConversationActivityFact::WorkPhase{work_id, kind}` | Paged command output and a cancel intent |
| Permissions and questions | `app/lib/widget/agent_chat/permission_card.dart`, `question_card.dart` | `agent-chat-permissions-v1`, `permission-policy-v1` | Pending decisions delivered inside the ordered projection rather than by a separate read |
| Subagents | `app/lib/widget/agent_chat/sub_agent_view.dart`, `sub_agent_box.dart` | `ConversationActivityFact::SubagentStarted{child_id, parent_tool_use_id}` | Child task text, model, live activity and output; never an invented child transcript |
| Header chips | `app/lib/widget/agent_chat/header_chips.dart`, `chat_stats.dart` | `ConversationActivityFact::ContextUsage` only | `status-chip` catalog records for files, processes, timeline, tasks, subagents, errors, permissions, plan, sync |
| Composer | `app/lib/widget/agent_chat/input_bar.dart`, `panel_send_message.dart` | `agent-chat-intents-v1`, `attachments-v1` | Draft stays client-local; intents and attachment transfer stay Gentd-owned |
| Provider/model/effort/mode | `app/lib/widget/agent_chat/input_chips_row.dart`, `panel_build.dart` | `AgentChatSelection` create/switch; no display records | The catalogs from backlog item 2 |
| MCP tools and connectors | `app/lib/provider/connector_provider.dart`, `app/lib/service/mcp_client_manager.dart` | `forge-connectors-v1`, plus a startup JSON at `<data_dir>/standalone-mcp.json` | Source registration, enabled state, lifecycle, health, per-run tool activity |
| Git, branch, worktree | `app/lib/provider/repo_cache_provider.dart` and Git services | Not chat-projected | Workspace projection with branch, worktree, dirty/add/delete counts |
| Canvas | native UI | Intentionally native-only | Excluded |

## Existing IPC foundations

- Native Gentd client: `app/lib/service/gentd/gentd_ipc_client.dart`
- Native Gentd reducer, to be replaced: `app/lib/provider/agent_chat/agent_chat_gentd.dart`
- Rust conversation reads: `crates/gent-cli/src/chat_cli/reads.rs`
- Rust activity reads: `crates/gent-cli/src/conversation_activity.rs`
- Rust attachment transfer: `crates/gent-cli/src/chat_cli/attachments.rs`
- Rust selection and prompt intents: `crates/gent-cli/src/chat_cli.rs`
- Gentd runtime facade: `crates/gentd/src/runtime_facade.rs` (struct) and
  `runtime_facade_api.rs` (`RuntimeApi` impl). The advertised capability list is not there — it is
  built by `declared_capabilities_with_profiles()` in `crates/gent-runtime/src/catalog.rs:193` and
  wired at `crates/gentd/src/runtime_facade_composition.rs:36-39`.

## Smart metadata behavior to preserve

The native app requests a title after the first assistant completion and a recap at completions 6,
12, 18 and so on. It supplies a bounded durable transcript rather than resuming the interactive
provider session, asks the provider's cheap summary model for JSON-only title and recap fields, and
persists usable results. Gentd must own this schedule for Claude, Codex and Claurst with one
provider-neutral artifact provenance. Neither client may derive a substitute title or recap from a
truncated message.

## Projection shape

Defined once, in `native-agent-chat-cutover-map.md` under "Projection contract to add first". Do not
restate it here.

Conversation summaries carry title, recap, preview, update time, message count, workspace, current
provider/model/effort/mode, lifecycle and attention state. The snapshot carries normalized transcript
entries, activity facts, decision state, current run and workspace facts. Cursor ordering is
Gentd-owned.

## Integration approach

Decided: extend `gentd_ipc_client.dart` and reduce projections into view models. One authority,
realtime, no CLI subprocess. Schema-generated Dart/Rust DTOs remain available if the protocol surface
grows; mirroring Gentd facts into retained native state does not, because it keeps two sources of
truth.

## Acceptance before native work

See the gate in `README.md`. It supersedes the criteria previously listed here.
