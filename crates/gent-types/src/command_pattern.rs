use crate::{
    CommandGrant, CommandPatternError, PermissionCategory, PermissionDecisionResponse,
    PermissionDecisionResponseKind, PermissionRequest,
};
use serde_json::Value;

const MAX_WORDS: usize = 32;
const MAX_PATTERN_BYTES: usize = 1024;
const MAX_TOOL_NAME_BYTES: usize = 128;
const SHELLS: [&str; 4] = ["sh", "bash", "zsh", "dash"];
const LAUNCHERS: [&str; 34] = [
    "sh",
    "bash",
    "zsh",
    "dash",
    "fish",
    "ksh",
    "cmd",
    "powershell",
    "pwsh",
    "env",
    "sudo",
    "doas",
    "su",
    "xargs",
    "eval",
    "exec",
    "nohup",
    "time",
    "timeout",
    "nice",
    "command",
    "builtin",
    "busybox",
    "python",
    "python3",
    "node",
    "perl",
    "ruby",
    "php",
    "deno",
    "bun",
    "npx",
    "iex",
    "invoke-expression",
];

struct Word {
    text: String,
    bare: bool,
}

pub fn answered_command_grant(
    request: &PermissionRequest,
    response: &PermissionDecisionResponse,
) -> Result<Option<CommandGrant>, CommandPatternError> {
    match (response.response, response.command_pattern.as_deref()) {
        (PermissionDecisionResponseKind::ApproveCommandPattern, Some(pattern)) => {
            command_grant_for(request, pattern).map(Some)
        }
        (PermissionDecisionResponseKind::ApproveCommandPattern, None) => {
            Err(CommandPatternError::Missing)
        }
        (_, Some(_)) => Err(CommandPatternError::Unexpected),
        (_, None) => Ok(None),
    }
}

pub fn command_grant_for(
    request: &PermissionRequest,
    pattern: &str,
) -> Result<CommandGrant, CommandPatternError> {
    let argv = request_command(request)?.ok_or(CommandPatternError::NotMatchable)?;
    let grant = parse_command_pattern(&request.tool_name, pattern)?;
    if matches(&grant, &argv) {
        Ok(grant)
    } else {
        Err(CommandPatternError::Mismatch)
    }
}

pub fn command_grant_permits(grant: &CommandGrant, request: &PermissionRequest) -> bool {
    grant.tool_name == request.tool_name
        && matches!(request_command(request), Ok(Some(argv)) if matches(grant, &argv))
}

pub fn validate_command_grant(grant: &CommandGrant) -> Result<(), CommandPatternError> {
    let Some(first) = grant.words.first() else {
        return Err(CommandPatternError::TooBroad);
    };
    let tool_valid = !grant.tool_name.is_empty()
        && grant.tool_name.len() <= MAX_TOOL_NAME_BYTES
        && grant.tool_name.bytes().all(|byte| byte.is_ascii_graphic());
    let bytes: usize = grant.words.iter().map(|word| word.len() + 1).sum();
    if !tool_valid
        || grant.words.len() > MAX_WORDS
        || bytes > MAX_PATTERN_BYTES
        || first.is_empty()
        || first.contains('=')
        || !grant.words.iter().all(|word| literal_word(word))
    {
        return Err(CommandPatternError::Invalid);
    }
    if grant.any_arguments && launches_commands(first) {
        return Err(CommandPatternError::TooBroad);
    }
    Ok(())
}

pub fn parse_command_pattern(
    tool_name: &str,
    pattern: &str,
) -> Result<CommandGrant, CommandPatternError> {
    if pattern.len() > MAX_PATTERN_BYTES {
        return Err(CommandPatternError::Invalid);
    }
    let mut words = simple_words(pattern).ok_or(CommandPatternError::Invalid)?;
    let any_arguments = words
        .last()
        .is_some_and(|word| word.bare && word.text == "*");
    if any_arguments {
        words.pop();
    }
    let grant = CommandGrant {
        tool_name: tool_name.to_owned(),
        words: words.into_iter().map(|word| word.text).collect(),
        any_arguments,
    };
    validate_command_grant(&grant)?;
    Ok(grant)
}

fn request_command(
    request: &PermissionRequest,
) -> Result<Option<Vec<String>>, CommandPatternError> {
    if request.category != PermissionCategory::Command {
        return Err(CommandPatternError::NotCommand);
    }
    let argv = match request
        .input
        .as_ref()
        .and_then(|input| input.get("command"))
    {
        Some(Value::String(text)) => texts(simple_words(text)),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>(),
        _ => return Err(CommandPatternError::NotCommand),
    };
    Ok(argv.and_then(unwrap_shell).filter(|argv| !argv.is_empty()))
}

fn unwrap_shell(argv: Vec<String>) -> Option<Vec<String>> {
    match argv.as_slice() {
        [shell, flag, script]
            if SHELLS.contains(&program_name(shell).as_str())
                && matches!(flag.as_str(), "-c" | "-lc") =>
        {
            texts(simple_words(script))
        }
        _ => Some(argv),
    }
}

fn matches(grant: &CommandGrant, argv: &[String]) -> bool {
    argv.len() >= grant.words.len()
        && (grant.any_arguments || argv.len() == grant.words.len())
        && argv.iter().zip(&grant.words).all(|(arg, word)| arg == word)
}

fn texts(words: Option<Vec<Word>>) -> Option<Vec<String>> {
    words.map(|words| words.into_iter().map(|word| word.text).collect())
}

fn literal_word(word: &str) -> bool {
    !word.starts_with('~')
        && word
            .chars()
            .all(|c| (c.is_ascii_graphic() || c == ' ') && !matches!(c, '*' | '?' | '[' | ']'))
}

fn launches_commands(first: &str) -> bool {
    LAUNCHERS.contains(&program_name(first).as_str())
}

fn program_name(word: &str) -> String {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let lower = name.to_ascii_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_owned()
}

fn simple_words(text: &str) -> Option<Vec<Word>> {
    if !text
        .chars()
        .all(|c| c.is_ascii() && (!c.is_ascii_control() || matches!(c, ' ' | '\t' | '\n' | '\r')))
    {
        return None;
    }
    let bytes = text.as_bytes();
    let mut words = Vec::new();
    let mut index = 0;
    loop {
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
            index += 1;
        }
        let Some(&first) = bytes.get(index) else {
            return Some(words);
        };
        let (word, next) = if matches!(first, b'\'' | b'"') {
            quoted(bytes, index)?
        } else {
            bare(bytes, index)?
        };
        if next < bytes.len() && !matches!(bytes[next], b' ' | b'\t') {
            return None;
        }
        words.push(word);
        index = next;
    }
}

fn quoted(bytes: &[u8], start: usize) -> Option<(Word, usize)> {
    let quote = bytes[start];
    let length = bytes[start + 1..].iter().position(|&byte| byte == quote)?;
    let content = &bytes[start + 1..start + 1 + length];
    if quote == b'"'
        && content
            .iter()
            .any(|byte| matches!(byte, b'$' | b'`' | b'\\'))
    {
        return None;
    }
    let text = String::from_utf8(content.to_vec()).ok()?;
    Some((Word { text, bare: false }, start + length + 2))
}

fn bare(bytes: &[u8], start: usize) -> Option<(Word, usize)> {
    if bytes[start] == b'@' {
        return None;
    }
    let mut index = start;
    while index < bytes.len() && !matches!(bytes[index], b' ' | b'\t') {
        match bytes[index] {
            b';' | b'&' | b'|' | b'<' | b'>' | b'(' | b')' | b'{' | b'}' | b'$' | b'`' | b'#'
            | b'\n' | b'\r' | b'\'' | b'"' => return None,
            b'\\'
                if !bytes.get(index + 1).is_some_and(|next| {
                    next.is_ascii_alphanumeric() || matches!(next, b'.' | b'_' | b'-')
                }) =>
            {
                return None;
            }
            _ => index += 1,
        }
    }
    let text = String::from_utf8(bytes[start..index].to_vec()).ok()?;
    if text == "--%" {
        return None;
    }
    Some((Word { text, bare: true }, index))
}

#[cfg(test)]
#[path = "command_pattern_tests.rs"]
mod tests;
