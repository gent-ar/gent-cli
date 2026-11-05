use std::io::{ErrorKind, Read, Seek, SeekFrom};

const MAGIC: [u8; 4] = *b"GGUF";
const MAX_METADATA_BYTES: u64 = 256 * 1024 * 1024;
const MAX_KEYS: u64 = 65_536;
const MAX_KEY_BYTES: u64 = 65_536;
const MAX_KEPT_TEXT_BYTES: u64 = 4_096;
const MAX_NUMERIC_KEYS: usize = 64;
const NUMERIC_SUFFIXES: [&str; 7] = [
    ".context_length",
    ".block_count",
    ".embedding_length",
    ".attention.head_count",
    ".attention.head_count_kv",
    ".attention.key_length",
    ".attention.value_length",
];

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct GgufMetadata {
    pub(crate) architecture: String,
    pub(crate) name: Option<String>,
    pub(crate) has_chat_template: bool,
    numeric: Vec<(String, u64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GgufError {
    NotGguf,
    Unreadable,
}

enum Value {
    Number(u64),
    Text { bytes: u64, kept: Option<String> },
    Other,
}

impl GgufMetadata {
    pub(crate) fn architecture_value(&self, suffix: &str) -> Option<u64> {
        let key = format!("{}{suffix}", self.architecture);
        self.numeric
            .iter()
            .find_map(|(name, value)| (*name == key).then_some(*value))
    }
}

pub(crate) fn read_metadata(mut source: impl Read + Seek) -> Result<GgufMetadata, GgufError> {
    let length = source.seek(SeekFrom::End(0)).map_err(read_error)?;
    source.seek(SeekFrom::Start(0)).map_err(read_error)?;
    let mut reader = Reader {
        inner: source,
        consumed: 0,
        limit: length.min(MAX_METADATA_BYTES),
    };
    if reader.array::<4>()? != MAGIC || !matches!(reader.u32()?, 2 | 3) {
        return Err(GgufError::NotGguf);
    }
    reader.u64()?;
    let keys = reader.u64()?;
    if keys > MAX_KEYS {
        return Err(GgufError::NotGguf);
    }
    let mut metadata = GgufMetadata::default();
    for _ in 0..keys {
        let key = reader.key()?;
        let kind = reader.u32()?;
        let keep = key == "general.architecture" || key == "general.name";
        match (key.as_str(), reader.value(kind, keep, 0)?) {
            (
                "general.architecture",
                Value::Text {
                    kept: Some(text), ..
                },
            ) => {
                metadata.architecture = text;
            }
            ("general.name", Value::Text { kept, .. }) => metadata.name = kept,
            ("tokenizer.chat_template", Value::Text { bytes, .. }) => {
                metadata.has_chat_template = bytes > 0;
            }
            (key, Value::Number(value))
                if metadata.numeric.len() < MAX_NUMERIC_KEYS
                    && NUMERIC_SUFFIXES.iter().any(|suffix| key.ends_with(suffix)) =>
            {
                metadata.numeric.push((key.to_owned(), value));
            }
            _ => {}
        }
    }
    if metadata.architecture.trim().is_empty() {
        return Err(GgufError::NotGguf);
    }
    Ok(metadata)
}

struct Reader<R> {
    inner: R,
    consumed: u64,
    limit: u64,
}

impl<R: Read + Seek> Reader<R> {
    fn charge(&mut self, bytes: u64) -> Result<(), GgufError> {
        self.consumed = self
            .consumed
            .checked_add(bytes)
            .filter(|total| *total <= self.limit)
            .ok_or(GgufError::NotGguf)?;
        Ok(())
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], GgufError> {
        self.charge(N as u64)?;
        let mut bytes = [0; N];
        self.inner.read_exact(&mut bytes).map_err(read_error)?;
        Ok(bytes)
    }

    fn u32(&mut self) -> Result<u32, GgufError> {
        self.array::<4>().map(u32::from_le_bytes)
    }

    fn u64(&mut self) -> Result<u64, GgufError> {
        self.array::<8>().map(u64::from_le_bytes)
    }

    fn skip(&mut self, bytes: u64) -> Result<(), GgufError> {
        self.charge(bytes)?;
        let offset = i64::try_from(bytes).map_err(|_| GgufError::NotGguf)?;
        self.inner
            .seek(SeekFrom::Current(offset))
            .map_err(read_error)?;
        Ok(())
    }

    fn bounded_text(&mut self, bytes: u64) -> Result<String, GgufError> {
        self.charge(bytes)?;
        let mut text = Vec::new();
        (&mut self.inner)
            .take(bytes)
            .read_to_end(&mut text)
            .map_err(read_error)?;
        if text.len() as u64 != bytes {
            return Err(GgufError::NotGguf);
        }
        String::from_utf8(text).map_err(|_| GgufError::NotGguf)
    }

    fn key(&mut self) -> Result<String, GgufError> {
        let bytes = self.u64()?;
        if bytes == 0 || bytes > MAX_KEY_BYTES {
            return Err(GgufError::NotGguf);
        }
        self.bounded_text(bytes)
    }

    fn value(&mut self, kind: u32, keep: bool, depth: u8) -> Result<Value, GgufError> {
        Ok(match kind {
            0 => Value::Number(u64::from(self.array::<1>()?[0])),
            2 => Value::Number(u64::from(u16::from_le_bytes(self.array::<2>()?))),
            4 => Value::Number(u64::from(self.u32()?)),
            10 => Value::Number(self.u64()?),
            1 | 7 => self.array::<1>().map(|_| Value::Other)?,
            3 => self.array::<2>().map(|_| Value::Other)?,
            5 | 6 => self.array::<4>().map(|_| Value::Other)?,
            11 | 12 => self.array::<8>().map(|_| Value::Other)?,
            8 => {
                let bytes = self.u64()?;
                if keep && bytes <= MAX_KEPT_TEXT_BYTES {
                    let text = self.bounded_text(bytes)?;
                    Value::Text {
                        bytes,
                        kept: Some(text),
                    }
                } else {
                    self.skip(bytes)?;
                    Value::Text { bytes, kept: None }
                }
            }
            9 if depth == 0 => self.list()?,
            _ => return Err(GgufError::NotGguf),
        })
    }

    fn list(&mut self) -> Result<Value, GgufError> {
        let kind = self.u32()?;
        let count = self.u64()?;
        let mut largest = None;
        for _ in 0..count {
            match self.value(kind, false, 1)? {
                Value::Number(value) => largest = largest.max(Some(value)),
                Value::Text { .. } | Value::Other => {}
            }
        }
        Ok(largest.map_or(Value::Other, Value::Number))
    }
}

fn read_error(error: std::io::Error) -> GgufError {
    if error.kind() == ErrorKind::UnexpectedEof {
        GgufError::NotGguf
    } else {
        GgufError::Unreadable
    }
}

#[cfg(test)]
#[path = "local_model_gguf_tests.rs"]
pub(crate) mod tests;
