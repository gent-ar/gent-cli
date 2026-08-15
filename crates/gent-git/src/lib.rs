//! Pure Git porcelain parsing and typed outcomes. Process execution is composed by `gentd` later.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusEntry {
    pub index_status: char,
    pub worktree_status: char,
    pub path: String,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum GitError {
    #[error("invalid porcelain v1 status line")]
    InvalidPorcelain,
}

/// Parses `git status --porcelain=v1 -z` output without shelling out.
///
/// # Errors
/// Returns an error when a record cannot be interpreted safely.
pub fn parse_porcelain_v1_z(output: &[u8]) -> Result<Vec<StatusEntry>, GitError> {
    output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .map(parse_record)
        .collect()
}

fn parse_record(record: &[u8]) -> Result<StatusEntry, GitError> {
    if record.len() < 4 || record[2] != b' ' {
        return Err(GitError::InvalidPorcelain);
    }
    let path = std::str::from_utf8(&record[3..]).map_err(|_| GitError::InvalidPorcelain)?;
    Ok(StatusEntry {
        index_status: char::from(record[0]),
        worktree_status: char::from(record[1]),
        path: path.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::{GitError, StatusEntry, parse_porcelain_v1_z};

    #[test]
    fn parses_spaces_and_multiple_records_without_shell_heuristics() {
        let parsed = parse_porcelain_v1_z(b"M  staged.rs\0?? space name.txt\0").unwrap();
        assert_eq!(
            parsed,
            vec![
                StatusEntry {
                    index_status: 'M',
                    worktree_status: ' ',
                    path: "staged.rs".into()
                },
                StatusEntry {
                    index_status: '?',
                    worktree_status: '?',
                    path: "space name.txt".into()
                }
            ]
        );
    }

    #[test]
    fn rejects_malformed_records() {
        assert_eq!(
            parse_porcelain_v1_z(b"broken\0"),
            Err(GitError::InvalidPorcelain)
        );
    }
}
