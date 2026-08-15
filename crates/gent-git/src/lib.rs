//! Pure Git porcelain parsing and typed outcomes. Process execution is composed by `gentd` later.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusEntry {
    pub index_status: char,
    pub worktree_status: char,
    pub path: String,
    /// Original path for a rename or copy entry; absent for all other states.
    pub original_path: Option<String>,
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
    let mut records = output.split(|byte| *byte == 0).peekable();
    let mut entries = Vec::new();
    while let Some(record) = records.next() {
        if record.is_empty() {
            return records
                .peek()
                .is_none()
                .then_some(entries)
                .ok_or(GitError::InvalidPorcelain);
        }
        let (mut entry, has_original_path) = parse_record(record)?;
        if has_original_path {
            entry.original_path = Some(parse_path(
                records.next().ok_or(GitError::InvalidPorcelain)?,
            )?);
        }
        entries.push(entry);
    }
    Ok(entries)
}

fn parse_record(record: &[u8]) -> Result<(StatusEntry, bool), GitError> {
    if record.len() < 4 || record[2] != b' ' {
        return Err(GitError::InvalidPorcelain);
    }
    let index_status = char::from(record[0]);
    let worktree_status = char::from(record[1]);
    let entry = StatusEntry {
        index_status,
        worktree_status,
        path: parse_path(&record[3..])?,
        original_path: None,
    };
    Ok((
        entry,
        matches!(index_status, 'R' | 'C') || matches!(worktree_status, 'R' | 'C'),
    ))
}

fn parse_path(bytes: &[u8]) -> Result<String, GitError> {
    let path = std::str::from_utf8(bytes).map_err(|_| GitError::InvalidPorcelain)?;
    (!path.is_empty())
        .then_some(path.into())
        .ok_or(GitError::InvalidPorcelain)
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
                    path: "staged.rs".into(),
                    original_path: None,
                },
                StatusEntry {
                    index_status: '?',
                    worktree_status: '?',
                    path: "space name.txt".into(),
                    original_path: None,
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

    #[test]
    fn parses_renames_and_copies_with_their_reversed_original_path() {
        let parsed =
            parse_porcelain_v1_z(b"R  renamed.rs\0original.rs\0C  copied.rs\0source.rs\0").unwrap();
        assert_eq!(
            parsed,
            vec![
                StatusEntry {
                    index_status: 'R',
                    worktree_status: ' ',
                    path: "renamed.rs".into(),
                    original_path: Some("original.rs".into()),
                },
                StatusEntry {
                    index_status: 'C',
                    worktree_status: ' ',
                    path: "copied.rs".into(),
                    original_path: Some("source.rs".into()),
                },
            ]
        );
    }

    #[test]
    fn rejects_a_rename_without_its_original_path() {
        assert_eq!(
            parse_porcelain_v1_z(b"R  renamed.rs\0"),
            Err(GitError::InvalidPorcelain)
        );
    }

    #[test]
    fn rejects_empty_path_records_without_consuming_the_next_entry() {
        assert_eq!(
            parse_porcelain_v1_z(b"R  renamed.rs\0\0M  later.rs\0"),
            Err(GitError::InvalidPorcelain)
        );
    }
}
