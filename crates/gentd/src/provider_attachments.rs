use std::path::Path;

use gent_ports::{AttachmentBlobStore, AttachmentLedger};
use gent_types::AttachmentMetadata;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedTurnAttachment {
    pub(crate) metadata: AttachmentMetadata,
    bytes: Vec<u8>,
}

pub(crate) fn resolve<L, B>(
    ledger: &L,
    blobs: &B,
    turn_id: &str,
) -> Result<Vec<ResolvedTurnAttachment>, String>
where
    L: AttachmentLedger,
    B: AttachmentBlobStore,
{
    ledger
        .turn_attachments(turn_id)
        .map_err(|_| "turn attachments are unavailable".to_owned())?
        .into_iter()
        .map(|metadata| {
            let bytes = blobs
                .read_attachment_blob(&metadata.storage_key)
                .map_err(|_| "attachment content is unavailable".to_owned())?;
            (bytes.len() as u64 == metadata.byte_len
                && hex::encode(Sha256::digest(&bytes)) == metadata.digest_sha256)
                .then_some(ResolvedTurnAttachment { metadata, bytes })
                .ok_or_else(|| "attachment content verification failed".to_owned())
        })
        .collect()
}

pub(crate) fn claude_content(attachments: &[ResolvedTurnAttachment]) -> Vec<Value> {
    attachments
        .iter()
        .filter(|attachment| image(&attachment.metadata.media_type))
        .map(|attachment| {
            json!({"type":"image","source":{"type":"base64","media_type":attachment.metadata.media_type,"data":base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &attachment.bytes)}})
        })
        .collect()
}

pub(crate) fn prompt_with_files(prompt: &str, attachments: &[ResolvedTurnAttachment]) -> String {
    const MAX_TEXT_BYTES: usize = 64 * 1024;
    let mut value = prompt.to_owned();
    let mut remaining = MAX_TEXT_BYTES;
    for attachment in attachments
        .iter()
        .filter(|attachment| !image(&attachment.metadata.media_type))
    {
        value.push_str("\n\nAttached file: ");
        value.push_str(&attachment.metadata.display_name);
        match std::str::from_utf8(&attachment.bytes) {
            Ok(text) if text.len() <= remaining => {
                value.push_str("\n```text\n");
                value.push_str(text);
                value.push_str("\n```");
                remaining -= text.len();
            }
            Ok(_) => value.push_str("\nText content exceeds the attachment input limit."),
            Err(_) => {
                value.push_str("\nBinary content is attached but cannot be represented as text.")
            }
        }
    }
    value
}

pub(crate) fn claurst_prompt_with_files(
    prompt: &str,
    attachments: &[gent_ports::ClaurstPromptAttachment],
) -> String {
    const MAX_TEXT_BYTES: usize = 64 * 1024;
    let mut value = prompt.to_owned();
    let mut remaining = MAX_TEXT_BYTES;
    for attachment in attachments
        .iter()
        .filter(|attachment| !image(&attachment.media_type))
    {
        value.push_str("\n\nAttached file: ");
        value.push_str(&attachment.display_name);
        match std::str::from_utf8(&attachment.bytes) {
            Ok(text) if text.len() <= remaining => {
                value.push_str("\n```text\n");
                value.push_str(text);
                value.push_str("\n```");
                remaining -= text.len();
            }
            Ok(_) => value.push_str("\nText content exceeds the attachment input limit."),
            Err(_) => {
                value.push_str("\nBinary content is attached but cannot be represented as text.")
            }
        }
    }
    value
}

pub(crate) fn claurst_images(
    attachments: &[gent_ports::ClaurstPromptAttachment],
) -> Vec<gent_ports::ClaurstPromptAttachment> {
    attachments
        .iter()
        .filter(|attachment| image(&attachment.media_type))
        .cloned()
        .collect()
}

pub(crate) fn codex_local_images(
    attachments: &[ResolvedTurnAttachment],
    root: &Path,
) -> Result<Vec<Value>, String> {
    std::fs::create_dir_all(root).map_err(|_| "provider attachment storage is unavailable")?;
    attachments
        .iter()
        .filter(|attachment| image(&attachment.metadata.media_type))
        .map(|attachment| {
            let path = root.join(&attachment.metadata.digest_sha256);
            std::fs::write(&path, &attachment.bytes)
                .map_err(|_| "provider attachment storage is unavailable")?;
            Ok(json!({"type":"localImage","path":path}))
        })
        .collect()
}

fn image(media_type: &str) -> bool {
    matches!(
        media_type,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use gent_types::AttachmentMetadata;
    use serde_json::json;
    use sha2::{Digest, Sha256};

    use super::{ResolvedTurnAttachment, claude_content, codex_local_images, prompt_with_files};

    fn image(bytes: &[u8]) -> ResolvedTurnAttachment {
        ResolvedTurnAttachment {
            metadata: AttachmentMetadata {
                attachment_id: "attachment-a".into(),
                display_name: "image.png".into(),
                media_type: "image/png".into(),
                byte_len: bytes.len() as u64,
                digest_sha256: hex::encode(Sha256::digest(bytes)),
                storage_key: "sha256/image".into(),
            },
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn claude_images_are_base64_content_blocks() {
        let attachments = vec![image(b"png")];

        assert_eq!(
            claude_content(&attachments),
            vec![
                json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"cG5n"}})
            ]
        );
    }

    #[test]
    fn codex_images_are_materialized_only_under_the_owned_root() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("owned");
        let attachments = vec![image(b"png")];

        let encoded = codex_local_images(&attachments, &root).unwrap();
        let path = encoded[0]["path"].as_str().unwrap();

        assert_eq!(encoded[0]["type"], "localImage");
        assert!(Path::new(path).starts_with(&root));
        assert_eq!(std::fs::read(path).unwrap(), b"png");
        assert!(!encoded[0].to_string().contains("attachment-a"));
    }

    #[test]
    fn text_attachments_are_visible_to_the_model_input() {
        let mut attachment = image(b"notes");
        attachment.metadata.media_type = "text/plain".into();
        attachment.metadata.display_name = "notes.txt".into();

        assert!(prompt_with_files("Read this", &[attachment]).contains("notes.txt"));
        assert!(prompt_with_files("Read this", &[image(b"png")]).ends_with("Read this"));
    }
}
