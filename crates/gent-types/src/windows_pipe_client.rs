use crate::windows_pipe_name;
use std::path::Path;

fn ordinary_absolute_win32(value: &str) -> bool {
    let bytes = value.as_bytes();
    let separator = |byte: u8| matches!(byte, b'\\' | b'/');
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && separator(bytes[2])
    {
        return true;
    }
    if bytes.len() < 5 || !separator(bytes[0]) || !separator(bytes[1]) || separator(bytes[2]) {
        return false;
    }
    let mut components = value[2..].split(['\\', '/']);
    let (Some(server), Some(share)) = (components.next(), components.next()) else {
        return false;
    };
    !server.is_empty()
        && !share.is_empty()
        && !matches!(server, "." | "?" | "??")
        && !server
            .chars()
            .any(|c| c.is_control() || matches!(c, ':' | '?'))
        && !matches!(share, "." | "..")
}

pub fn windows_pipe_client_candidates(data_dir: &Path) -> Vec<String> {
    let mut names = vec![windows_pipe_name(data_dir)];
    let Some(spelling) = data_dir.to_str() else {
        return names;
    };
    if !ordinary_absolute_win32(spelling) {
        return names;
    }
    for spelling in [spelling.replace('/', "\\"), spelling.replace('\\', "/")] {
        let name = windows_pipe_name(Path::new(&spelling));
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

#[cfg(test)]
#[path = "windows_pipe_client_tests.rs"]
mod tests;
