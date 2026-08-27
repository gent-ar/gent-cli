use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Result},
    path::Path,
};

pub(crate) fn file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8_192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            return Ok(hex::encode(hasher.finalize()));
        }
        hasher.update(&buffer[..count]);
    }
}

pub(crate) fn matches_sha256(path: &Path, expected: &str) -> Result<bool> {
    Ok(file_sha256(path)? == expected)
}
