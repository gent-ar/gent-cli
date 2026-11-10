use gent_types::ToolPhase;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;

#[path = "inline_diff.rs"]
mod diff;

#[derive(Clone, Debug, Default)]
pub(super) struct ToolView {
    pub(super) name: String,
    pub(super) input: Option<Value>,
    pub(super) phase: Option<ToolPhase>,
    pub(super) output: String,
    pub(super) settled: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Kind {
    Read,
    Run,
    Edit,
    Search,
    Other,
}

fn kind(name: &str) -> Kind {
    match name.to_ascii_lowercase().as_str() {
        "read" | "read_file" | "view" | "notebookread" => Kind::Read,
        "bash" | "powershell" | "shell" | "command" | "commandexecution" | "run_shell_command"
        | "exec_command" | "local_shell" => Kind::Run,
        "edit" | "multiedit" | "write" | "filechange" | "apply_patch" | "edit_file"
        | "write_file" | "str_replace" | "create" | "notebookedit" => Kind::Edit,
        "grep" | "glob" | "search" | "find" | "rg" | "search_files" => Kind::Search,
        _ => Kind::Other,
    }
}

impl ToolView {
    fn field(&self, keys: &[&str]) -> Option<String> {
        let input = self.input.as_ref()?;
        keys.iter()
            .find_map(|key| input.get(*key).and_then(Value::as_str))
            .or_else(|| {
                input
                    .get("changes")?
                    .as_array()?
                    .first()?
                    .get("path")?
                    .as_str()
            })
            .map(str::to_owned)
    }

    fn label(&self) -> String {
        match (kind(&self.name), self.name.to_ascii_lowercase().as_str()) {
            (Kind::Run, "powershell") => "PowerShell".into(),
            (Kind::Run, _) => "Bash".into(),
            (Kind::Read, _) => "Read".into(),
            (Kind::Edit, "write" | "write_file" | "create") => "Write".into(),
            (Kind::Edit, _) => "Edit".into(),
            (Kind::Search, "glob") => "Glob".into(),
            (Kind::Search, _) => "Grep".into(),
            (Kind::Other, _) => self.name.clone(),
        }
    }

    fn target(&self) -> String {
        let value = match kind(&self.name) {
            Kind::Run => self
                .field(&["command", "cmd"])
                .map(|value| command_text(&value)),
            Kind::Read | Kind::Edit => self
                .field(&["file_path", "path", "notebook_path"])
                .map(|value| short_path(&value)),
            Kind::Search => self
                .field(&["pattern", "query"])
                .map(|value| format!("\"{value}\"")),
            Kind::Other => self.field(&[
                "command",
                "file_path",
                "path",
                "query",
                "pattern",
                "url",
                "description",
                "prompt",
            ]),
        };
        value
            .map(|value| first_line(&value).to_owned())
            .unwrap_or_default()
    }

    fn running(&self) -> bool {
        !self.settled && matches!(self.phase, None | Some(ToolPhase::Started))
    }

    fn result(&self) -> (String, Color) {
        match self.phase {
            Some(ToolPhase::WaitingPermission) if !self.settled => {
                ("needs permission".into(), Color::Yellow)
            }
            Some(ToolPhase::Failed) => (self.failure(), Color::Red),
            Some(ToolPhase::Completed) => (self.success(), Color::Green),
            _ if self.running() => ("running…".into(), Color::Cyan),
            _ => ("interrupted".into(), Color::DarkGray),
        }
    }

    fn success(&self) -> String {
        let lines = output_lines(&self.output);
        match kind(&self.name) {
            Kind::Read => format!("({} line{})", lines.len(), plural(lines.len())),
            Kind::Run => "✓ exit 0".into(),
            Kind::Edit => {
                let (added, removed) = diff::counts(&self.diff());
                format!("+{added} -{removed}")
            }
            Kind::Search => format!("{} match{}", lines.len(), es(lines.len())),
            Kind::Other => "✓".into(),
        }
    }

    fn failure(&self) -> String {
        exit_code(&self.output).map_or_else(|| "✗ failed".into(), |code| format!("✗ exit {code}"))
    }

    fn diff(&self) -> Vec<(char, String)> {
        let input = self.input.as_ref();
        let text = |key: &str| {
            input
                .and_then(|value| value.get(key))
                .and_then(Value::as_str)
        };
        if let (Some(old), Some(new)) = (text("old_string"), text("new_string")) {
            return diff::lines(old, new);
        }
        if let Some(content) = text("content") {
            return diff::lines("", content);
        }
        let patch = input
            .and_then(|value| value.get("changes"))
            .and_then(Value::as_array)
            .map(|changes| {
                changes
                    .iter()
                    .filter_map(|change| change.get("diff").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .or_else(|| text("diff").or_else(|| text("patch")).map(str::to_owned))
            .unwrap_or_else(|| self.output.clone());
        diff::patch(&patch)
    }
}

pub(super) fn tool_lines(tool: &ToolView) -> Vec<Line<'static>> {
    let (result, color) = tool.result();
    let mut header = vec![
        Span::styled("● ", Style::default().fg(color)),
        Span::styled(tool.label(), Style::default().add_modifier(Modifier::BOLD)),
    ];
    let target = tool.target();
    if !target.is_empty() {
        header.push(Span::raw(format!(" {target}")));
    }
    header.push(Span::styled(
        format!("  {result}"),
        Style::default().fg(color),
    ));
    let mut lines = vec![Line::from(header)];
    let completed = matches!(tool.phase, Some(ToolPhase::Completed | ToolPhase::Failed));
    let applied = tool.phase == Some(ToolPhase::Completed);
    match kind(&tool.name) {
        Kind::Edit if applied => lines.extend(diff::rendered(&tool.diff())),
        Kind::Run if completed => {
            if let Some(tail) = output_lines(&tool.output)
                .into_iter()
                .rev()
                .find(|line| exit_code(line).is_none())
                .filter(|line| !(line.starts_with('(') && line.ends_with("with no output)")))
            {
                lines.push(Line::styled(
                    format!("  ⎿ {tail}"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        _ => {}
    }
    lines
}

pub(super) fn proposed_change(
    name: &str,
    input: Option<&Value>,
) -> (Option<String>, Vec<Line<'static>>) {
    let view = ToolView {
        name: name.into(),
        input: input.cloned(),
        ..ToolView::default()
    };
    let lines = if kind(name) == Kind::Edit
        || input.is_some_and(|value| value.get("old_string").is_some())
    {
        diff::rendered(&view.diff())
    } else {
        Vec::new()
    };
    (view.field(&["file_path", "path", "notebook_path"]), lines)
}

fn output_lines(output: &str) -> Vec<&str> {
    output
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect()
}

fn exit_code(text: &str) -> Option<i64> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("Exit code "))
        .and_then(|code| code.trim().parse().ok())
}

pub(super) fn command_text(value: &str) -> String {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    if let Some(index) = lower.find("powershell.exe\" -command ")
        && trimmed.starts_with('"')
    {
        let rest = trimmed[index + "powershell.exe\" -command ".len()..].trim();
        return rest
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                rest.strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(rest)
            .replace(r"\\", r"\");
    }
    trimmed.to_owned()
}

fn short_path(value: &str) -> String {
    let value = value.strip_prefix(r"\\?\").unwrap_or(value);
    std::env::current_dir()
        .ok()
        .and_then(|cwd| {
            let cwd = cwd.to_string_lossy().into_owned();
            let cwd = cwd.strip_prefix(r"\\?\").unwrap_or(&cwd).to_owned();
            let head = value.get(..cwd.len())?;
            (head.eq_ignore_ascii_case(&cwd)).then(|| {
                value[cwd.len()..]
                    .trim_start_matches(['\\', '/'])
                    .to_owned()
            })
        })
        .filter(|relative| !relative.is_empty())
        .unwrap_or_else(|| value.to_owned())
}

fn first_line(value: &str) -> &str {
    value.lines().next().unwrap_or_default().trim()
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn es(count: usize) -> &'static str {
    if count == 1 { "" } else { "es" }
}
