# Conversation orchestration

A Gent conversation can create other conversations, address them, and wait on them. Gentd owns
creation, linkage, delivery and waiting. Nothing here is a second agent framework, a background
job, or a sub-agent: every conversation a conversation creates is an ordinary durable conversation
the user can open, with its own transcript, run, selection and provider process.

## What a linked conversation is

A link is one durable row in `agent_chat_conversation_links`
(`crates/gent-store/src/sqlite/conversation_link_ledger.rs`):

| Column | Meaning |
| --- | --- |
| `child_conversation_id` | Primary key: a conversation has at most one parent, forever |
| `parent_conversation_id` | The conversation that created it |
| `label` | The short, user-facing name the creator gave the child |
| `created_run_id` / `created_turn_id` | The parent turn that created it |
| `created_at_unix_seconds` | Creation order within a thread |

A child is never reparented. Recording the same link twice is idempotent; recording a different
parent for an existing child is a typed invariant failure. Nothing else about a linked conversation
differs from a root conversation, so every existing restore, projection, permission, selection and
provider path applies to it unchanged.

A child inherits its parent's workspace and its parent's provider/model/effort/mode selection
unless the caller overrides `workspacePath` or `selection`. A conversation may only address
conversations in its own workspace.

The label is also the child's title from the moment it exists. At creation Gentd writes it as the
child's ordinary completed title artifact — the same durable record and the same
`ConversationArtifactLedger` write the LLM summarizer uses — so every client reads it through the
`title` it already reads, with no client-side rule about link notices. Only the child is titled, and
only then; the parent's own title is never touched. The existing title precedence applies unchanged:
`scheduled_requests` asks for a title only while a conversation has no completed title artifact, so
a labelled child is never later retitled from its content, exactly as a summarized conversation is
never retitled once its first title lands.

## The four facts

Link activity is published as ordinary `ConversationActivityFact` variants
(`crates/gent-types/src/conversation_activity.rs`) through the same append-only activity ledger
and the same projection stream as every other fact, so a client folds them with the reducer it
already has and a client that does not know these kinds still restores the conversation from the
same pages.

| Fact | Appended to | Carries |
| --- | --- | --- |
| `conversationCreated` | the **parent** | `childConversationId`, `label` (the child's), `workspacePath`, `selection`, `originToolUseId` |
| `createdByConversation` | the **child** | `parentConversationId`, `parentRunId`, `label` (the **parent's**) |
| `conversationMessageSent` | the **sender** | `targetConversationId`, `targetLabel`, `delivery`, `preview` |
| `conversationWaitSettled` | the **waiter** | `targets` (each `conversationId`, `label`, `phase`), `timedOut` |

Every fact names the conversations it talks about **and their labels**, so a card renders without
resolving names from earlier facts. That matters when the activity window is trimmed and when a
conversation addresses a sibling it did not create. A label is resolved from the target's own link
row, falling back to its conversation title; `label`, `targetLabel` and a target's `label` are
omitted from the wire only when genuinely unknown, and every non-identifying field is tolerant on
read.

## The intents

`crates/gent-protocol/src/agent_chat_intent.rs`, gated on the `conversation-links-v1` capability
and validated by `validate_conversation_link_request`:

| Request | Reply | Typed refusals |
| --- | --- | --- |
| `createLinkedConversation` | `linkedConversationCreated` | `conversationLabelInvalid`, `conversationMessageEmpty`, `conversationNotFound` |
| `sendToConversation` | `conversationMessageDelivered` | `conversationSelfTargeted`, `conversationMessageEmpty`, `conversationNotFound`, `conversationForeignWorkspace` |
| `waitForConversations` | `conversationWaitSettled` | `conversationSelfTargeted`, `conversationWaitTargetsInvalid`, `conversationNotFound`, `conversationForeignWorkspace` |
| `listLinkedConversations` | `conversationLinks` | `conversationNotFound` |

A conversation may never address or await itself, and never a conversation it cannot name, so a
wait can neither deadlock nor observe another workspace. `waitForConversations` is bounded by
`MAX_CONVERSATION_WAIT_SECONDS` (900) and runs off the transport thread. On timeout it returns
`Ok` with `timedOut: true` and one honest result per requested conversation — each target's real
phase and last reply — instead of an error.

## The tools

The chat domain of Gent's internal MCP server (`gent mcp chat`) exposes four tools
(`crates/gent-cli/src/chat_mcp_tools.rs`), which are protocol clients like every other Gent CLI
surface:

- `gent_chat_create` — start a new chat with a label, an optional opening prompt and an optional
  workspace. Returns its `conversationId`.
- `gent_chat_send` — say something to a chat this one created or was created by.
- `gent_chat_wait` — block until named chats finish, or until `timeoutSeconds` elapses.
- `gent_chat_list` — recover the thread (parent and children with labels and phases) after a
  restart.

The model never learns or states its own conversation id. Gentd launches the chat MCP server for a
provider process with `--conversation-id <that provider launch's conversation>`, and the tools
resolve the caller from that binding (`crates/gentd/src/conversation_scoped_mcp.rs`). Only the
`gent-chat` server is scoped; other internal servers are launched unchanged.

Each provider gets that scoping on its own launch path:

- **Claude** — every launch writes `<data dir>/conversation-mcp/<run id>.json`, containing the
  selected servers with `gent-chat` scoped, and passes it as `--mcp-config`. The global
  `standalone-mcp.json` is never handed to a provider and is never mutated. The per-run file is
  removed when the run exits or is released.
- **Codex** — gentd reads the configured servers, selects and scopes them, and hands the exact
  value to the launch as `CodexPromptStart.mcp_servers`. The driver no longer selects servers.
- **Claurst** — both the settings roster and the launch roster are scoped as named entries, and the
  conversation is part of the runtime's session identity, so a conversation gets its own Claurst
  agent rather than reusing one bound to another conversation.

### The Claurst runtime splits at the model/session seam

Claurst's MCP roster is per **process**, not per ACP session, so a conversation cannot simply be
given its own roster on `session/new`. That was verified rather than assumed. `session/new` accepts
an `mcpServers` list, but Claurst 0.1.7 discards it (`src-rust/crates/acp/src/server.rs`,
`on_new_session`: *"v1: ignore req.mcp_servers — agent uses settings.json MCP roster"*), building
one `McpManager` per runtime in `runtime.rs::build_mcp_manager`. Driving the real staged binary
confirms it: two `session/new` calls on one live agent, each naming a different `gent-chat` command
line, both succeed and both log `ACP: session-specific MCP servers are not yet routed (v1) — using
global config`. A per-session roster would scope nothing and silently leave every conversation
sharing the first one's chat server.

What a conversation switch does **not** need is a model reload. A Claurst runtime is two processes:
llama.cpp, which holds the model in RAM, and the Claurst ACP agent, which is cheap. The launch plan
(`crates/gentd/src/claurst_local_runtime.rs`) splits cleanly along that seam:

| Process | Determined by |
| --- | --- |
| llama.cpp argv | the model (path, `--ctx-size`), `effort` (`qwen3_reasoning_arguments`), and `effort`+`mode` (`--chat-template-kwargs`) |
| Claurst ACP | `claurst_home` and `LLAMA_CPP_HOST` — and its `settings.json` roster, which carries `--conversation-id` |

Workspace, permission mode, MCP digest, tool sources and conversation never reach a llama argument.
So `RuntimeIdentity` (`crates/gentd/src/standalone_claurst_runtime_identity.rs`) is two parts, and
the reuse decision is three-way:

- **model identity** (model, effort, mode) differs → retire both processes and relaunch.
- **session identity** (workspace, permission mode, MCP digest, tool sources, conversation) differs
  alone → rewrite `settings.json` and replace only the ACP agent. llama.cpp keeps running and the
  model stays resident.
- everything matches → reuse as is.

`ClaurstStandaloneOwner::rebind_session` performs the second case against the runtime's existing
port, and refuses with `ModelPlanChanged` if the recomputed llama plan is not byte-identical, so
the seam can never silently hand a conversation a runtime loaded for a different model. Ordering
matters and is pinned by tests: the chat template is materialized before llama.cpp starts, because
llama.cpp reads `--chat-template-file` at launch, while `settings.json` is materialized just before
the ACP agent that reads it.

This also fixes a cost that predates orchestration: switching permission mode, tool sources or
workspace used to reload the model too, and none of those change a single llama argument.

When Claurst routes `session/new`'s `mcpServers`, this should be revisited again: drop
`conversation_id` from the session identity and give each conversation's ACP session its own roster
through `initialize_session_with_mcp`, which already exists for exactly that shape. A conversation
switch would then cost nothing at all.

## Delivery rides the one queue

There is exactly one way to put a prompt into a conversation, and cross-conversation delivery uses
it (`crates/gentd/src/runtime_facade_conversation_links_delivery.rs`):

- target **idle** → `sendPrompt` → `delivery: "queued"`; the message starts the target's next turn.
- target **running** → `queuePrompt` then `steerQueuedPrompt` → `delivery: "steered"`; the message
  enters the turn in flight exactly as a user steer does, with each provider's native steering.

No second ingress, no direct provider write, no bypass of admission, permissions, goals or
receipts. Whatever a user could send by hand, a conversation sends the same way, and every
durable consequence is identical.

## Restart guarantees

Everything above is durable before it is reported. The link row, the facts and the delivered
prompt are written in the ledger, so after a restart:

- `gent_chat_list` recovers a thread from the link rows alone; no in-memory state is consulted.
- Facts already appended keep their cursors and replay to any client from cursor zero.
- A delivered message is an ordinary durable prompt, so it is recovered by the ordinary prompt
  dispatch recovery.
- A wait does not survive a restart, because it is a read loop and never durable state. The caller
  simply waits again; the targets' phases are read fresh from the ledger each poll.
