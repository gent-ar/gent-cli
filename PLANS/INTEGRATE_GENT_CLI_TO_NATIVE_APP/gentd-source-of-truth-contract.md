# Gentd source-of-truth contract

## Rule

Gentd is the only agent-chat product authority. The `gent` terminal and the native app are presentation clients over the same daemon, data directory, snapshot/delta streams and intent API.

The native app must not contain a second provider adapter, a second mode vocabulary, a permission-policy mapping, a model catalog, a tool parser, a provider readiness state machine, a conversation repair path or durable chat state. It does not translate Gent concepts into native enums.

## Generic native UI contract

Gentd publishes display-ready, stable-ID records. Flutter renders generic controls from those records and sends their IDs back unchanged.

None of these records exist yet. Today every vocabulary is a bare Rust enum — `AgentChatProvider`, `AgentChatEffort`, `AgentChatMode` in `crates/gent-types/src/agent_chat.rs`, `PermissionMode` and `PermissionCategory` in `crates/gent-types/src/policies.rs` — carrying no label, ordering, availability or explanation. The single exception is `LocalModel.label` (`crates/gent-protocol/src/local_models.rs:18`). The table below is therefore a build specification, not a description.

One generic record type serves every row: `id`, `label`, `ordering`, `available`, `unavailable_reason`, `explanation`, `requires_confirmation`, `scope`. Catalogs are addressed by catalog ID (`provider`, `model`, `effort`, `mode`, `permission-policy`, `tool-source`, `composer-action`, `status-chip`), so a new vocabulary adds an ID rather than a frame type. Declare the capability in `crates/gent-runtime/src/catalog.rs`.

| Gentd projection | Generic native rendering |
| --- | --- |
| Provider catalog | Provider picker with label, icon reference, readiness, availability and selection ID |
| Model catalog | Model picker grouped by provider with label, capability badges, installed/download state and selection ID |
| Effort catalog | Picker populated from `id`, `label`, ordering, availability and current ID |
| Mode catalog | Mode chip/menu populated from `id`, `label`, ordering, availability, explanation and current ID |
| Permission-policy catalog | Independent Permissions chip/menu populated from policy records, risk explanation, confirmation requirement and current ID |
| Tool-source/MCP catalog | Tools chip populated from source ID, label, enabled state, health and actions |
| Header/status catalog | Generic status chips from daemon-provided label, count, severity, action ID and target reference |
| Timeline/activity records | Generic typed timeline rows with stable entity ID, presentation kind, phase, title, detail, output reference and supported actions |
| Conversation/session/sidebar records | Generic rows from daemon-provided title, subtitle, status, timestamp, attention and action IDs |
| Composer actions | Generic action descriptors such as attach, draw, save prompt, Git, checkpoint and provider login |

Adding a supported effort, mode, permission policy, model, MCP source, status chip or action must therefore change Gentd and become visible in both clients without a Flutter release. Flutter only changes when Gent introduces an entirely new presentation primitive that cannot be expressed through the established generic record types.

## Intent rule

Flutter sends an intent name plus the daemon-issued IDs and an idempotency receipt. It never derives a provider command, shell argument, permission behavior or continuation from a label.

```text
intent = operation ID + target IDs + expected revision + receipt ID
result = accepted receipt | revision conflict | typed unavailable/failed state
```

The daemon validates capability, scope and revision. It returns the updated projection through the normal ordered stream. Flutter updates its visual state from that stream, not by locally applying an inferred provider result.

## Cutover consequence

Remove all native hard-coded values for `default`, `plan`, `autonomous`, `auto-accept edits`, `bypass permissions`, provider-specific approval mapping and adapter process control. The replacement is a generic Mode control and a generic independent Permissions control driven solely by Gentd catalog records.

The same removal applies on the terminal side, which is done first: `crates/gent-cli/src/terminal/state_permissions.rs` parses the literals `ask | read | edits | autonomous | bypass confirm`, while `crates/gent-cli/src/terminal/render_composer.rs:109` displays a different set, `ask | read-only | auto edits | autonomous | bypass`. Both literal sets are deleted; the parser matches catalog IDs and the renderer prints catalog labels.

The same removal applies to provider/model availability, tool categorization, MCP readiness, local-model download ownership, titles/recaps, session lifecycle, tool/subagent reconstruction, checkpoint behavior and provider authentication.
