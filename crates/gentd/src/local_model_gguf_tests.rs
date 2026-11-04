use std::io::Cursor;

use super::{GgufError, read_metadata};

#[derive(Default)]
pub(crate) struct Gguf {
    pub(crate) magic: Option<[u8; 4]>,
    pub(crate) version: Option<u32>,
    pub(crate) key_count: Option<u64>,
    entries: Vec<u8>,
    count: u64,
}

impl Gguf {
    pub(crate) fn text(mut self, key: &str, value: &str) -> Self {
        self.key(key, 8);
        self.string(value);
        self
    }

    pub(crate) fn number(mut self, key: &str, value: u32) -> Self {
        self.key(key, 4);
        self.entries.extend(value.to_le_bytes());
        self
    }

    pub(crate) fn token_list(mut self, key: &str, tokens: &[&str]) -> Self {
        self.key(key, 9);
        self.entries.extend(8_u32.to_le_bytes());
        self.entries.extend((tokens.len() as u64).to_le_bytes());
        for token in tokens {
            self.string(token);
        }
        self
    }

    pub(crate) fn number_list(mut self, key: &str, values: &[u32]) -> Self {
        self.key(key, 9);
        self.entries.extend(4_u32.to_le_bytes());
        self.entries.extend((values.len() as u64).to_le_bytes());
        for value in values {
            self.entries.extend(value.to_le_bytes());
        }
        self
    }

    pub(crate) fn bytes(&self) -> Vec<u8> {
        let mut bytes = self.magic.unwrap_or(*b"GGUF").to_vec();
        bytes.extend(self.version.unwrap_or(3).to_le_bytes());
        bytes.extend(0_u64.to_le_bytes());
        bytes.extend(self.key_count.unwrap_or(self.count).to_le_bytes());
        bytes.extend(&self.entries);
        bytes
    }

    fn key(&mut self, key: &str, kind: u32) {
        self.count += 1;
        self.string(key);
        self.entries.extend(kind.to_le_bytes());
    }

    fn string(&mut self, value: &str) {
        self.entries.extend((value.len() as u64).to_le_bytes());
        self.entries.extend(value.as_bytes());
    }
}

pub(crate) fn chat_model(name: &str, context: u32) -> Gguf {
    Gguf::default()
        .text("general.architecture", "llama")
        .text("general.name", name)
        .number("llama.context_length", context)
        .number("llama.block_count", 16)
        .number_list("llama.attention.head_count_kv", &[4, 8])
        .token_list("tokenizer.ggml.tokens", &["a", "b", "c"])
        .text("tokenizer.chat_template", "{{ messages }}")
}

#[test]
fn reads_bounded_metadata_from_a_v3_header() {
    let metadata = read_metadata(Cursor::new(chat_model("Tiny Llama", 8_192).bytes())).unwrap();
    assert_eq!(metadata.architecture, "llama");
    assert_eq!(metadata.name.as_deref(), Some("Tiny Llama"));
    assert!(metadata.has_chat_template);
    assert_eq!(metadata.architecture_value(".context_length"), Some(8_192));
    assert_eq!(metadata.architecture_value(".block_count"), Some(16));
    assert_eq!(
        metadata.architecture_value(".attention.head_count_kv"),
        Some(8)
    );
    assert_eq!(metadata.architecture_value(".embedding_length"), None);
}

#[test]
fn a_model_without_a_template_parses_without_one() {
    let metadata = read_metadata(Cursor::new(
        Gguf::default()
            .text("general.architecture", "qwen3")
            .number("qwen3.context_length", 4_096)
            .bytes(),
    ))
    .unwrap();
    assert!(!metadata.has_chat_template);
    assert_eq!(metadata.name, None);
}

#[test]
fn rejects_bad_magic_versions_and_truncation() {
    let mut bad_magic = chat_model("m", 8_192);
    bad_magic.magic = Some(*b"GGML");
    let mut version_one = chat_model("m", 8_192);
    version_one.version = Some(1);
    let mut truncated = chat_model("m", 8_192).bytes();
    truncated.truncate(truncated.len() - 3);
    for bytes in [
        bad_magic.bytes(),
        version_one.bytes(),
        truncated,
        Vec::new(),
    ] {
        assert_eq!(read_metadata(Cursor::new(bytes)), Err(GgufError::NotGguf));
    }
}

#[test]
fn never_trusts_untrusted_counts_or_lengths() {
    let mut many_keys = chat_model("m", 8_192);
    many_keys.key_count = Some(u64::MAX);
    assert_eq!(
        read_metadata(Cursor::new(many_keys.bytes())),
        Err(GgufError::NotGguf)
    );
    let with_tail = |tail: &[u8]| {
        let mut header = Gguf::default().text("general.architecture", "llama");
        header.key_count = Some(2);
        let mut bytes = header.bytes();
        bytes.extend(tail);
        read_metadata(Cursor::new(bytes))
    };
    let mut endless_array = 1_u64.to_le_bytes().to_vec();
    endless_array.push(b'k');
    endless_array.extend(9_u32.to_le_bytes());
    endless_array.extend(4_u32.to_le_bytes());
    endless_array.extend(u64::MAX.to_le_bytes());
    let mut huge_name = 12_u64.to_le_bytes().to_vec();
    huge_name.extend(b"general.name");
    huge_name.extend(8_u32.to_le_bytes());
    huge_name.extend(u64::MAX.to_le_bytes());
    let mut nested = 1_u64.to_le_bytes().to_vec();
    nested.push(b'k');
    nested.extend(9_u32.to_le_bytes());
    nested.extend(9_u32.to_le_bytes());
    nested.extend(1_u64.to_le_bytes());
    for tail in [
        u64::MAX.to_le_bytes().to_vec(),
        endless_array,
        huge_name,
        nested,
    ] {
        assert_eq!(with_tail(&tail), Err(GgufError::NotGguf));
    }
}

#[test]
fn requires_an_architecture() {
    assert_eq!(
        read_metadata(Cursor::new(
            Gguf::default()
                .number("llama.context_length", 8_192)
                .bytes()
        )),
        Err(GgufError::NotGguf)
    );
}
