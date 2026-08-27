# Gentd automation and Forge port contract

Domain detail behind backlog item 5. Paths are relative to
`/Users/ivanmatiasfort/Clouseau/clouseau-app` unless prefixed `crates/`, which is
`/Users/ivanmatiasfort/Clouseau/gent-cli`.

## Automations

Native source: `app/lib/model/automation_definition.dart`, `app/lib/provider/automation_engine.dart`,
`app/lib/provider/automation_provider.dart`.

Gentd already declares `automations-v1` (`crates/gent-runtime/src/catalog.rs`) with types in
`crates/gent-types/src/automations.rs` and `crates/gent-protocol/src/automations.rs`. Audit those
against the shape below and add only what is missing; do not create a parallel domain.

| Domain | Required durable shape |
| --- | --- |
| Definition | ID, name, working directory, enabled state, action, trigger, optional condition, provider selection, chain target, notification preferences, timestamps, last run state |
| Action | Prompt, skill, skill plus follow-up prompt, or explicit script |
| Trigger | Manual, schedule, webhook, or file-watch with event set and debounce |
| Run | ID, automation ID, optional conversation ID, start/end, status, summary, safe error, condition result, triggering parent |
| Lifecycle | Running, success, error, cancelled, skipped; bounded history per automation |
| Chain | Explicit next automation ID with cycle and depth protection |

The scheduler creates ordinary Gent conversations and runs through the same durable prompt ingress as
an interactive client. It must not use provider-native session IDs or a Flutter-owned queue.

Acceptance: an automation triggered from the terminal produces a conversation the native app lists
without either client writing its own run record.

## Forge and MCP connectors

Native source: `app/lib/provider/connector/connector_registry.dart`,
`app/lib/util/mcp_handshake.dart`.

Gentd already declares `forge-connectors-v1` (types in `crates/gent-protocol/src/forge.rs`). Forge
registers a generated stdio MCP connector after validation; Gentd owns the connector record, safe
public metadata, validation state, enabled state and the discovered tool catalog. Both clients render
those records through one projection.

| Field | Public projection | Private daemon data |
| --- | --- | --- |
| Connector | ID, name, description, category, phase, declared and discovered tool names | command, args, environment, credential values |
| Validation | phase, safe diagnostic category, tool count | protocol frames, process output |
| Enablement | workspace- or conversation-scoped enabled state | launch configuration |

Generated Forge connectors use the normal Gent MCP configuration path after validation; they are not
terminal-local subprocesses.

`forge-connectors-v1` is extended to cover every MCP source, not only Forge-generated ones — config
source registration, live updates, health, credential ownership, per-conversation selection and
reconnect semantics. `<data_dir>/standalone-mcp.json`
(`crates/gentd/src/standalone_mcp_config.rs`), which registers the internal `gent-automations` and
`gent-forge` servers at startup, becomes one registered source rather than the mechanism. Do not add
a second MCP capability beside it.

Acceptance: a connector registered from either client appears in both with the same health and
enabled state, and its credentials never cross the projection boundary.

## Deferred native integration

Flutter becomes a thin client for the same automation and Forge projections. Canvas stays
native-only.
