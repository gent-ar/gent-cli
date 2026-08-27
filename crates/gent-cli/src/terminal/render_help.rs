use ratatui::{
    style::{Color, Modifier, Style},
    text::Line,
};

pub(super) fn lines() -> Vec<Line<'static>> {
    [
        (
            "Start",
            "Ctrl+N creates a chat in the current workspace; a focused session receives that new chat.",
        ),
        (
            "Navigate",
            "↑/↓ chooses a chat. /search TEXT filters titles and recaps. PgUp/PgDn reads history.",
        ),
        (
            "Send",
            "Type a message and press Enter. Shift+Enter or Alt+Enter adds a line. /resume reopens the selected chat.",
        ),
        (
            "Templates",
            "/templates opens a picker; /template ID name=value renders a backed prompt.",
        ),
        (
            "Sessions",
            "/session NAME creates a durable session for this chat. Ctrl+G focuses sessions; Enter opens its newest chat.",
        ),
        (
            "Documents",
            "/documents opens a picker; Enter stages the selected workspace document.",
        ),
        (
            "Tools",
            "/tools lists configured MCP servers. All configured servers are active by default.",
        ),
        (
            "Git",
            "/git shows the selected workspace, branch, and changed-file count.",
        ),
        (
            "Selection",
            "Tab provider · Ctrl+L model · Ctrl+E effort · Ctrl+O mode. Enter applies a context-preserving switch; Ctrl+P changes permissions; Ctrl+G opens sessions.",
        ),
        (
            "Login",
            "/login starts the selected Claude or Codex provider's own login flow. Gent's local models need no login.",
        ),
        (
            "Context",
            "Ctrl+X toggles preserved or cleared context for the next model, effort, mode, or provider switch. /fork applies the current selection explicitly.",
        ),
        (
            "Files",
            "Drag or paste a file path, or use /attach PATH. /detach clears pending files.",
        ),
        (
            "Decisions",
            "/approve once, /approve-tool, /approve-category, /deny, or /answer JSON handles a request.",
        ),
        (
            "Permissions",
            "Ctrl+P changes the workspace posture: ask every action, read-only, auto-approve edits, autonomous, or bypass all. It does not change Ask, Plan, or Agent mode.",
        ),
        (
            "Goals",
            "/goal SUMMARY binds a goal to the current durable run.",
        ),
        (
            "Automation",
            "/automation or /automations opens Gent's automation picker; Enter runs the selected enabled automation in this chat.",
        ),
        (
            "Activity",
            "F2 or /activity opens the live tools, processes, subagents, permissions, and timeline.",
        ),
        (
            "Thinking",
            "Ctrl+T shows or summarizes provider-emitted thinking and saves that preference. Ctrl+C interrupts the active prompt.",
        ),
        ("Close", "F1, ?, or /help returns to the chat. Ctrl+Q quits."),
    ]
    .into_iter()
    .flat_map(|(label, detail)| {
        [
            Line::styled(
                label,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::from(format!("  {detail}")),
            Line::default(),
        ]
    })
    .collect()
}
