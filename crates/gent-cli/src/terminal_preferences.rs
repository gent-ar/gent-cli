use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Preferences {
    show_thinking: bool,
}

pub(crate) fn load(data_dir: &Path) -> Result<bool, String> {
    let path = path(data_dir);
    if !path.exists() {
        return Ok(false);
    }
    serde_json::from_slice::<Preferences>(&std::fs::read(path).map_err(|error| error.to_string())?)
        .map(|preferences| preferences.show_thinking)
        .map_err(|error| error.to_string())
}

pub(crate) fn save(data_dir: &Path, show_thinking: bool) -> Result<(), String> {
    std::fs::create_dir_all(data_dir).map_err(|error| error.to_string())?;
    let bytes =
        serde_json::to_vec(&Preferences { show_thinking }).map_err(|error| error.to_string())?;
    std::fs::write(path(data_dir), bytes).map_err(|error| error.to_string())
}

fn path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("ui-preferences.json")
}

#[cfg(test)]
mod tests {
    use super::{load, save};

    #[test]
    fn thinking_preference_survives_a_new_terminal_state() {
        let directory = tempfile::tempdir().unwrap();
        assert!(!load(directory.path()).unwrap());
        save(directory.path(), true).unwrap();
        assert!(load(directory.path()).unwrap());
    }
}
