use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandGrant {
    pub tool_name: String,
    pub words: Vec<String>,
    pub any_arguments: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
pub enum CommandPatternError {
    #[error("approveCommandPattern requires a commandPattern")]
    Missing,
    #[error("commandPattern is only accepted with approveCommandPattern")]
    Unexpected,
    #[error("the command pattern is not one simple command of literal words")]
    Invalid,
    #[error("the command pattern would approve arbitrary commands")]
    TooBroad,
    #[error("the pending request is not a command")]
    NotCommand,
    #[error("the pending command is compound or uses shell expansion; approve it once instead")]
    NotMatchable,
    #[error("the command pattern does not match the pending command")]
    Mismatch,
}

impl CommandPatternError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Missing => "permissionCommandPatternMissing",
            Self::Unexpected => "permissionCommandPatternUnexpected",
            Self::Invalid => "permissionCommandPatternInvalid",
            Self::TooBroad => "permissionCommandPatternTooBroad",
            Self::NotCommand => "permissionCommandPatternNotCommand",
            Self::NotMatchable => "permissionCommandNotMatchable",
            Self::Mismatch => "permissionCommandPatternMismatch",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandGrant, CommandPatternError};

    #[test]
    fn grant_wire_shape_is_closed_and_camel_case() {
        let grant: CommandGrant = serde_json::from_value(serde_json::json!({
            "toolName": "Bash", "words": ["npm", "run"], "anyArguments": true
        }))
        .unwrap();
        assert_eq!(grant.words, ["npm", "run"]);
        assert!(
            serde_json::from_value::<CommandGrant>(serde_json::json!({
                "toolName": "Bash", "words": [], "anyArguments": true, "regex": ".*"
            }))
            .is_err()
        );
        assert_eq!(
            CommandPatternError::NotMatchable.code(),
            "permissionCommandNotMatchable"
        );
    }
}
