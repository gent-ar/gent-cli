//! Filesystem-backed opaque attachment staging with content-addressed promotion.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use gent_ports::{AttachmentBlobStore, LedgerError};
use sha2::{Digest, Sha256};

/// Local content-addressed attachment byte store. It accepts only opaque SHA-256 keys.
#[derive(Clone, Debug)]
pub struct FileAttachmentBlobs {
    root: PathBuf,
}

impl FileAttachmentBlobs {
    /// Creates staging and final content directories below an application-owned root.
    ///
    /// # Errors
    /// Returns an error when the local directories cannot be created.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, LedgerError> {
        let store = Self { root: root.into() };
        fs::create_dir_all(store.root.join("staging")).map_err(io_error)?;
        fs::create_dir_all(store.root.join("blobs")).map_err(io_error)?;
        Ok(store)
    }

    fn staging(&self, key: &str) -> Result<PathBuf, LedgerError> {
        Ok(self.root.join("staging").join(digest(key)?))
    }
    fn final_path(&self, key: &str) -> Result<PathBuf, LedgerError> {
        Ok(self.root.join("blobs").join(digest(key)?))
    }
}

impl AttachmentBlobStore for FileAttachmentBlobs {
    fn append_attachment_chunk(
        &self,
        key: &str,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), LedgerError> {
        let path = self.staging(key)?;
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)
            .map_err(io_error)?;
        let actual = file.metadata().map_err(io_error)?.len();
        if actual
            == offset.saturating_add(
                u64::try_from(bytes.len()).map_err(|_| {
                    LedgerError::Invariant("attachment chunk length overflow".into())
                })?,
            )
        {
            let mut existing = vec![0; bytes.len()];
            file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
            file.read_exact(&mut existing).map_err(io_error)?;
            return if existing == bytes {
                Ok(())
            } else {
                Err(LedgerError::Invariant(
                    "attachment retry bytes differ from staged content".into(),
                ))
            };
        }
        if actual != offset {
            return Err(LedgerError::Invariant(
                "attachment blob offset differs from staged length".into(),
            ));
        }
        file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
        file.write_all(bytes).map_err(io_error)?;
        file.sync_data().map_err(io_error)
    }

    fn attachment_digest(&self, key: &str) -> Result<(u64, String), LedgerError> {
        let staging = self.staging(key)?;
        let path = if staging.exists() {
            staging
        } else {
            self.final_path(key)?
        };
        let mut file = File::open(path).map_err(io_error)?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 8192];
        let mut size = 0_u64;
        loop {
            let read = file.read(&mut buffer).map_err(io_error)?;
            if read == 0 {
                break;
            }
            size = size
                .checked_add(
                    u64::try_from(read)
                        .map_err(|_| LedgerError::Invariant("attachment size overflow".into()))?,
                )
                .ok_or_else(|| LedgerError::Invariant("attachment size overflow".into()))?;
            digest.update(&buffer[..read]);
        }
        Ok((size, format!("{:x}", digest.finalize())))
    }

    fn commit_attachment_blob(&self, key: &str) -> Result<(), LedgerError> {
        let staging = self.staging(key)?;
        let final_path = self.final_path(key)?;
        if final_path.exists() {
            verify_digest(&final_path, digest(key)?)?;
            if staging.exists() {
                fs::remove_file(staging).map_err(io_error)?;
            }
            return Ok(());
        }
        match fs::hard_link(&staging, &final_path) {
            Ok(()) => fs::remove_file(staging).map_err(io_error),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                verify_digest(&final_path, digest(key)?)?;
                fs::remove_file(staging).map_err(io_error)
            }
            Err(error) => Err(io_error(error)),
        }
    }
}

fn digest(key: &str) -> Result<&str, LedgerError> {
    let Some(value) = key.strip_prefix("sha256/") else {
        return Err(LedgerError::Invariant(
            "attachment storage key is not sha256-addressed".into(),
        ));
    };
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(value)
    } else {
        Err(LedgerError::Invariant(
            "attachment storage key has an invalid digest".into(),
        ))
    }
}
fn verify_digest(path: &Path, expected: &str) -> Result<(), LedgerError> {
    let mut file = File::open(path).map_err(io_error)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    if format!("{:x}", digest.finalize()) == expected {
        Ok(())
    } else {
        Err(LedgerError::Invariant(
            "existing attachment blob digest differs from its key".into(),
        ))
    }
}
fn io_error(error: impl std::fmt::Display) -> LedgerError {
    LedgerError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use gent_ports::AttachmentBlobStore;
    use sha2::{Digest, Sha256};

    use super::FileAttachmentBlobs;

    #[test]
    fn chunks_are_ordered_and_promotion_deduplicates_content() {
        let directory = tempfile::tempdir().unwrap();
        let bytes = b"hello";
        let digest = format!("{:x}", Sha256::digest(bytes));
        let key = format!("sha256/{digest}");
        let store = FileAttachmentBlobs::open(directory.path()).unwrap();
        store.append_attachment_chunk(&key, 0, b"he").unwrap();
        store.append_attachment_chunk(&key, 0, b"he").unwrap();
        store.append_attachment_chunk(&key, 2, b"llo").unwrap();
        assert_eq!(store.attachment_digest(&key).unwrap(), (5, digest));
        store.commit_attachment_blob(&key).unwrap();
        assert!(directory.path().join("blobs").join(&key[7..]).exists());
    }
}
