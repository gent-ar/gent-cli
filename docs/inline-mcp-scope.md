# Inline editor MCP scope

The editor persists full-server exclusions in the existing conversation `disallowedTools` configuration. A rule such as `mcp__gent-forge__*` means that every tool on that server is unavailable; Gentd now omits the whole server from the provider launch configuration. Partial tool restrictions retain the server.

Filtering happens after source selection and conversation scoping. Claude writes the filtered map to its per-run MCP file. Codex receives the filtered map in its launch options. Claurst filters both settings and launch entries and includes the conversation configuration revision in its MCP reuse identity. No global MCP source is modified, and conversations without exclusions retain their existing servers.

The app excludes its Forge, Canvas, Automations, Browser, Chat and Goal servers from inline editor requests. Project MCPs and native context/skills remain available. These exclusions reduce unnecessary tool exposure; they do not establish a security sandbox or guarantee faster provider first output.

Validation on Windows, 2026-10-23:

- Five shared conversation-scoping checks, five Claude MCP launch-file checks, one Codex launch-scoping check and five Claurst factory checks pass.
- The daemon builds successfully. An isolated Windows app used the patched daemon and produced an inline launch file containing zero app MCP servers; native skill access and replacement generation still worked.
- The app's focused parser, widget and remote configuration checks pass. Remote preparation persists all inline instructions and exclusions in one configuration save.
- The latest six-short-option paired request produced first text at 15.640 seconds and completed at 17.937 seconds. Earlier samples varied substantially; this is not a causal benchmark or a near-instant latency claim.

This worktree also contains the Windows cancellation repair described in `windows-inline-cancellation.md`. No commit, push, publication or replacement of the signed production runtime has been performed. Production packaging must include both runtime fixes.
