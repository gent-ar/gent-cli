# Native Agent Chat surface disposition

Implementation checklist for turning the native Agent Chat screen into a Gentd client. Each surface
has exactly one owner. Every path is relative to `/Users/ivanmatiasfort/Clouseau/clouseau-app`, and
was verified to exist; five entries in the previous revision named directories these files do not
live in and are corrected here.

"Catalog record" below means the generic record defined in `gentd-source-of-truth-contract.md`.
None of those records exist yet — backlog item 2 builds them.

## Composer

| Native surface | Current code | Future owner | Native behavior after cutover |
| --- | --- | --- | --- |
| Provider and model | `app/lib/widget/agent_chat/input_chips_row.dart`, `app/lib/widget/agent_chat/panel_composer_models.dart` | `provider` / `model` catalogs, selection intent | Render catalog entries in their `ordering`; submit `id` unchanged |
| Effort | `app/lib/widget/agent_chat/input_chips_row.dart`, `app/lib/widget/effort_picker_popover.dart` | `effort` catalog | Render available entries; no Dart effort enum |
| Mode | `app/lib/widget/agent_chat/input_chips_row.dart`, `app/lib/service/agent_chat/model_settings_controller.dart` | `mode` catalog | Render catalog entries; no Dart mode enum |
| Permissions | `app/lib/widget/agent_chat/permission_card.dart`, `app/lib/service/agent_chat/permission_controller.dart`, `app/lib/service/agent_chat/permission_coordinator.dart`, `app/lib/model/agent_adapter.dart` | `permission-policy` catalog, `PolicyRecord` revision, decision intents | Separate chip and cards; no provider-specific automatic approval |
| MCP/Tools | `app/lib/provider/connector_provider.dart`, `app/lib/service/mcp_client_manager.dart`, `app/lib/widget/agent_chat/input_chips_row.dart` | `tool-source` catalog and intents | Render enabled/health state; submit source IDs |
| Attach | `app/lib/widget/agent_chat/panel_attachments.dart` | `attachments-v1` staging | Native file picker stays local; bytes become a staged attachment receipt |
| Draw | `app/lib/widget/agent_chat/canvas_panel.dart` | Native canvas plus Gentd attachments | Canvas stays native-only; export is staged like any attachment |
| Save prompt | `app/lib/widget/agent_chat/panel_input.dart` | `prompt-templates-v1` | Native editor stays local; the record is Gentd-owned |
| Talk/STT | input bar and STT controls | Native device service | Transcribed text enters the normal prompt draft; no alternate agent path |
| Language picker | input controls | Native device service | Controls STT only |
| Git/worktree | `app/lib/provider/repo_cache_provider.dart` and Git services | Gentd workspace projection | Render branch/status; submit declared workspace actions |
| Checkpoint/fork/resume | `app/lib/widget/agent_chat/panel_dialogs.dart` | `agent-chat-intents-v1` | Render records; require an explicit confirmation receipt where filesystem state changes |
| Side question | `native:app/lib/provider/network/server/controller/agent_chat_btw.dart`, `native:app/lib/util/side_question_context.dart` | `agent-chat-side-question-v1` (new) | Decided: kept. Native issues `ask`/`cancel` intents and renders streamed `side_question_*` events; delete the Dart-side helper-process spawn, bounding and concurrency-cap code in the same cutover. See `integration-gap-review.md` for the specified shape |
| System prompt and advanced options | overflow/dialog controls | Gentd conversation configuration | Render descriptor-driven fields; persist nothing in Flutter |

## Conversation body and work views

| Native surface | Current code | Future owner | Native behavior after cutover |
| --- | --- | --- | --- |
| Messages and streaming | `app/lib/widget/agent_chat/message_list.dart`, `message_bubble.dart`, `streaming_bubble.dart` | Projection transcript | Render typed events and partial deltas in daemon cursor order |
| Thinking | `app/lib/widget/agent_chat/thinking_indicator.dart` | Projection activity plus a native visibility preference | The preference controls visibility only; it never filters daemon persistence |
| Tool timeline | `app/lib/widget/agent_chat/tool_timeline.dart`, `tool_output_view.dart`, `tool_presentation.dart` | Work-item projection | Render tool ID, phase, title, input/output/diff references and actions; no provider frame parsing |
| Commands/processes | `app/lib/widget/agent_chat/process_output_modal.dart` | Work-item projection | Render output pages; cancel only through a command intent |
| Subagents | `app/lib/widget/agent_chat/sub_agent_view.dart`, `sub_agent_box.dart` | Work-item projection | Render daemon-supplied child task, phase, output and actions |
| Permissions/questions | `app/lib/widget/agent_chat/permission_card.dart`, `question_card.dart` | Projection decision state | Render current decisions; submit immutable decision binding |
| Plan/tasks | plan modal, task chips | `reviewed-plan-v1` | Render provider-neutral records |
| Errors/interruption/loading | banners and indicators | Projection run/activity | Render the exact daemon phase and the actions it declares |

## Header and sidebar

| Native surface | Current code | Future owner | Native behavior after cutover |
| --- | --- | --- | --- |
| Header chips | `app/lib/widget/agent_chat/header_chips.dart`, `chat_stats.dart` (`AgentChatStats`) | `status-chip` catalog | Generic chip renderer: label, severity, count, target, action ID |
| Conversations | `app/lib/widget/agent_chat_conversation_sidebar.dart`, `app/lib/widget/agent_chat/sidebar/sidebar_conversations_sessions.dart`, `sidebar/conversation_row.dart`, `app/lib/provider/conversation_list_provider.dart` | Projection conversation summaries and search | Selection, search text and scroll stay local; every row field and action comes from Gentd |
| Sessions | `app/lib/widget/agent_chat/sidebar/sidebar_conversations_sessions.dart`, `app/lib/provider/session_index_provider.dart` | `agent-chat-sessions-v1`, folded into the projection | Pane state stays local; Gentd owns session records, order and active conversation |
| Prompts | `app/lib/widget/agent_chat/sidebar/sidebar_prompts_knowledge.dart`, `app/lib/provider/prompt_library_provider.dart` | `prompt-templates-v1` | Generic list/editor over template records and intents |
| Docs | `app/lib/widget/agent_chat/sidebar/sidebar_prompts_knowledge.dart`, `app/lib/util/knowledge_sources.dart` (`discoverKnowledgeFiles`) | `workspace-documents-v1` | Native open action consumes a daemon-issued reference |
| Activity | `app/lib/widget/agent_chat/sidebar/sidebar_activity.dart` | Projection workspace activity feed | Generic attention and running-work rows |
| Automations | `app/lib/widget/agent_chat/sidebar/sidebar_automations.dart`, `app/lib/provider/automation_provider.dart` | `automations-v1` | Generic definition and run-history UI plus intents |
| Canvas chip/panel | `app/lib/widget/agent_chat/canvas_panel.dart` | Native-only | Visibility is local; artifacts enter Gentd only as attachments |

## State that may remain native-local

- Selected sidebar section, modal/popover visibility, focus, keyboard shortcuts, scroll positions.
- Unsaved draft text and local-only STT/UI preferences.
- File-picker and canvas-editor transient state before an attachment is staged.
- Native visual theming and accessibility settings.

None of these may decide provider behavior, durable conversation state or permission policy.

## Source-first rule

Every row above is built in Gentd and consumed by the terminal before Flutter changes. Flutter
consumes the resulting catalogs; it never creates a replacement vocabulary or a compatibility
mapping. The terminal-side permission vocabulary that must change first is named in
`gentd-source-of-truth-contract.md`.
