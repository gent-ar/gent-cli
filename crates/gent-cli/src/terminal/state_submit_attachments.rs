use super::{UiEffect, UiState, notices::notice};

pub(super) fn attachment_command(
    state: &mut UiState,
    command: &str,
    argument: &str,
) -> Option<UiEffect> {
    if command == "/detach" {
        if !argument.is_empty() {
            return None;
        }
        let count = state.attachments.len();
        state.attachments.clear();
        state.input.clear();
        state.notice = Some(format!("Removed {count} pending attachment(s)."));
        return Some(UiEffect::Continue);
    }
    if argument.is_empty() {
        return Some(notice(state, "/attach requires a file path."));
    }
    Some(match attachment_path(argument) {
        Some(path) => attach(state, path),
        None => notice(state, "Attach requires a local file path."),
    })
}
pub(super) fn paste(state: &mut UiState, value: String) -> UiEffect {
    if let Some(path) = existing_file_path(&value) {
        attach(state, path)
    } else {
        state.input.push_str(&value);
        UiEffect::Continue
    }
}
pub(super) fn existing_file_path(value: &str) -> Option<std::path::PathBuf> {
    let path = attachment_path(value)?;
    path.is_file().then_some(path)
}

fn attachment_path(value: &str) -> Option<std::path::PathBuf> {
    let value = value.trim().trim_matches('"').replace("\\ ", " ");
    if let Some(value) = value.strip_prefix("file://") {
        return file_url_path(value).map(std::path::PathBuf::from);
    }
    (!value.is_empty()).then(|| std::path::PathBuf::from(value))
}

fn file_url_path(value: &str) -> Option<String> {
    let path = if value.starts_with('/') {
        value.to_owned()
    } else {
        format!("/{}", value.strip_prefix("localhost/")?)
    };
    let mut bytes = Vec::with_capacity(path.len());
    let source = path.as_bytes();
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'%' {
            let high = *source.get(index + 1)?;
            let low = *source.get(index + 2)?;
            bytes.push((hex(high)? << 4) | hex(low)?);
            index += 3;
        } else {
            bytes.push(source[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes).ok()
}

const fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
pub(super) fn attach(state: &mut UiState, path: std::path::PathBuf) -> UiEffect {
    if !path.is_file() {
        return notice(state, "Attach requires a readable file path.");
    }
    if state.attachments.iter().any(|value| value == &path) {
        return notice(state, "That file is already attached.");
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file")
        .to_owned();
    state.attachments.push(path);
    state.input.clear();
    state.notice = Some(format!("Attached {name}. Enter sends it with the prompt."));
    UiEffect::Continue
}
