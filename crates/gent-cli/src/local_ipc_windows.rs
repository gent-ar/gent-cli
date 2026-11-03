use std::path::Path;

pub(super) fn endpoint_absent(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(2 | 3))
}
pub(super) fn open_candidates<T>(
    data_dir: &Path,
    mut open: impl FnMut(&str) -> std::io::Result<T>,
) -> std::io::Result<T> {
    let mut absent = std::io::Error::from_raw_os_error(2);
    for candidate in gent_types::windows_pipe_client_candidates(data_dir) {
        match open(&candidate) {
            Ok(stream) => return Ok(stream),
            Err(error) if endpoint_absent(&error) => absent = error,
            Err(error) => return Err(error),
        }
    }
    Err(absent)
}

#[cfg(test)]
#[path = "local_ipc_windows_tests.rs"]
mod tests;
