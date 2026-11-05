use std::{
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use gent_protocol::model_catalog::LocalModelFileRejection;

use super::gguf::{GgufError, GgufMetadata, read_metadata};
use crate::local_model_catalog::LocalAgentProfile;

const MIN_CONTEXT_TOKENS: u64 = 4_096;
const MAX_CONTEXT_TOKENS: u64 = 32_768;
const COMPACT_BELOW_BYTES: u64 = 2_500_000_000;
const COMPUTE_RESERVE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UserModelFile {
    pub(crate) path: PathBuf,
    pub(crate) label: String,
    pub(crate) size_bytes: u64,
    pub(crate) modified_unix_nanos: u128,
    pub(crate) context_tokens: u32,
    pub(crate) runtime_memory_bytes: u64,
    pub(crate) agent_profile: LocalAgentProfile,
}

pub(crate) fn inspect(raw: &str) -> Result<UserModelFile, LocalModelFileRejection> {
    let requested = Path::new(raw);
    if !requested.is_absolute() || (cfg!(windows) && raw.starts_with(['\\', '/'])) {
        return Err(LocalModelFileRejection::UnsupportedPath);
    }
    if !requested
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
    {
        return Err(LocalModelFileRejection::NotGguf);
    }
    let listed =
        fs::symlink_metadata(requested).map_err(|_| LocalModelFileRejection::Unreadable)?;
    if listed.file_type().is_symlink() || !listed.is_file() {
        return Err(LocalModelFileRejection::UnsupportedPath);
    }
    let path =
        local_path(&fs::canonicalize(requested).map_err(|_| LocalModelFileRejection::Unreadable)?)?;
    let (size_bytes, modified_unix_nanos) =
        identity(&path).ok_or(LocalModelFileRejection::Unreadable)?;
    let metadata = read_metadata(BufReader::new(
        File::open(&path).map_err(|_| LocalModelFileRejection::Unreadable)?,
    ))
    .map_err(|error| match error {
        GgufError::NotGguf => LocalModelFileRejection::NotGguf,
        GgufError::Unreadable => LocalModelFileRejection::Unreadable,
    })?;
    if !metadata.has_chat_template {
        return Err(LocalModelFileRejection::MissingChatTemplate);
    }
    let context = metadata
        .architecture_value(".context_length")
        .ok_or(LocalModelFileRejection::NotGguf)?;
    if context < MIN_CONTEXT_TOKENS {
        return Err(LocalModelFileRejection::ContextTooSmall);
    }
    let context_tokens = context.min(MAX_CONTEXT_TOKENS);
    Ok(UserModelFile {
        label: label(&metadata, &path),
        size_bytes,
        modified_unix_nanos,
        context_tokens: u32::try_from(context_tokens).unwrap_or(u32::MAX),
        runtime_memory_bytes: size_bytes
            .saturating_add(size_bytes / 2)
            .saturating_add(kv_cache_bytes(&metadata, context_tokens).unwrap_or(size_bytes / 2))
            .saturating_add(COMPUTE_RESERVE_BYTES),
        agent_profile: if size_bytes < COMPACT_BELOW_BYTES {
            LocalAgentProfile::Compact
        } else {
            LocalAgentProfile::Full
        },
        path,
    })
}

pub(crate) fn identity(path: &Path) -> Option<(u64, u128)> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return None;
    }
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((metadata.len(), modified))
}

fn label(metadata: &GgufMetadata, path: &Path) -> String {
    metadata
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.len() <= 160 && !name.contains(char::is_control))
        .map(str::to_owned)
        .or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| stem.chars().take(160).collect())
        })
        .filter(|label: &String| !label.trim().is_empty())
        .unwrap_or_else(|| "Local model".into())
}

fn kv_cache_bytes(metadata: &GgufMetadata, context_tokens: u64) -> Option<u64> {
    let layers = metadata.architecture_value(".block_count")?;
    let heads = metadata.architecture_value(".attention.head_count");
    let kv_heads = metadata
        .architecture_value(".attention.head_count_kv")
        .or(heads)?;
    let per_head = match (
        metadata.architecture_value(".attention.key_length"),
        metadata.architecture_value(".attention.value_length"),
    ) {
        (Some(key), Some(value)) => key.checked_add(value)?,
        _ => metadata
            .architecture_value(".embedding_length")?
            .checked_mul(2)?
            .checked_div(heads?)?,
    };
    layers
        .checked_mul(context_tokens)?
        .checked_mul(kv_heads)?
        .checked_mul(per_head)?
        .checked_mul(17)
        .map(|bytes| bytes / 16)
}

#[cfg(windows)]
fn local_path(canonical: &Path) -> Result<PathBuf, LocalModelFileRejection> {
    use std::os::windows::fs::MetadataExt;
    let spelled = canonical.to_string_lossy();
    let drive = spelled.strip_prefix(r"\\?\").unwrap_or(&spelled);
    let bytes = drive.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || &bytes[1..3] != b":\\" {
        return Err(LocalModelFileRejection::UnsupportedPath);
    }
    let path = PathBuf::from(drive);
    for ancestor in path.ancestors() {
        let attributes = fs::symlink_metadata(ancestor)
            .map_err(|_| LocalModelFileRejection::Unreadable)?
            .file_attributes();
        if attributes & 0x400 != 0 {
            return Err(LocalModelFileRejection::UnsupportedPath);
        }
    }
    Ok(path)
}

#[cfg(not(windows))]
fn local_path(canonical: &Path) -> Result<PathBuf, LocalModelFileRejection> {
    Ok(canonical.to_path_buf())
}

#[cfg(test)]
#[path = "local_model_user_file_tests.rs"]
mod tests;
