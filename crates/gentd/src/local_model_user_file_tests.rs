use std::path::{Path, PathBuf};

use gent_protocol::model_catalog::LocalModelFileRejection;

use super::{identity, inspect};
use crate::{
    local_model_catalog::LocalAgentProfile,
    local_model_provisioning::gguf::tests::{Gguf, chat_model},
};

fn write(directory: &Path, name: &str, model: &Gguf) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, model.bytes()).unwrap();
    path
}

fn raw(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn derives_a_bounded_record_from_the_header_and_references_the_file_in_place() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(
        directory.path(),
        "tiny.gguf",
        &chat_model("Tiny Llama", 8_192),
    );
    let file = inspect(raw(&path)).unwrap();
    assert_eq!(file.label, "Tiny Llama");
    assert_eq!(file.context_tokens, 8_192);
    assert_eq!(file.agent_profile, LocalAgentProfile::Compact);
    assert_eq!(file.size_bytes, std::fs::metadata(&path).unwrap().len());
    assert!(file.runtime_memory_bytes > file.size_bytes + 512 * 1024 * 1024);
    assert_eq!(
        file.path.file_name().unwrap(),
        std::ffi::OsStr::new("tiny.gguf")
    );
    assert!(file.path.is_absolute());
    assert_eq!(
        identity(&file.path),
        Some((file.size_bytes, file.modified_unix_nanos))
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn clamps_long_contexts_and_falls_back_to_the_file_stem() {
    let directory = tempfile::tempdir().unwrap();
    let model = Gguf::default()
        .text("general.architecture", "qwen3")
        .number("qwen3.context_length", 131_072)
        .text("tokenizer.chat_template", "{{ x }}");
    let file = inspect(raw(&write(directory.path(), "My-Model.GGUF", &model))).unwrap();
    assert_eq!(file.context_tokens, 32_768);
    assert_eq!(file.label, "My-Model");
}

#[test]
fn rejects_unusable_files_with_typed_reasons() {
    let directory = tempfile::tempdir().unwrap();
    let no_template = Gguf::default()
        .text("general.architecture", "llama")
        .number("llama.context_length", 8_192);
    let mut bad_magic = chat_model("m", 8_192);
    bad_magic.magic = Some(*b"GGJT");
    let cases = [
        (
            write(directory.path(), "plain.gguf", &no_template),
            LocalModelFileRejection::MissingChatTemplate,
        ),
        (
            write(directory.path(), "small.gguf", &chat_model("m", 2_048)),
            LocalModelFileRejection::ContextTooSmall,
        ),
        (
            write(directory.path(), "magic.gguf", &bad_magic),
            LocalModelFileRejection::NotGguf,
        ),
        (
            write(directory.path(), "model.bin", &chat_model("m", 8_192)),
            LocalModelFileRejection::NotGguf,
        ),
        (
            directory.path().join("absent.gguf"),
            LocalModelFileRejection::Unreadable,
        ),
    ];
    for (path, rejection) in cases {
        assert_eq!(inspect(raw(&path)), Err(rejection), "{}", path.display());
    }
    std::fs::create_dir(directory.path().join("folder.gguf")).unwrap();
    assert_eq!(
        inspect(raw(&directory.path().join("folder.gguf"))),
        Err(LocalModelFileRejection::UnsupportedPath)
    );
    assert_eq!(
        inspect("relative/model.gguf"),
        Err(LocalModelFileRejection::UnsupportedPath)
    );
}

#[cfg(windows)]
#[test]
fn rejects_network_and_device_paths_on_windows() {
    for path in [
        r"\\server\share\model.gguf",
        r"\\?\UNC\server\share\model.gguf",
        r"\\.\PhysicalDrive0\model.gguf",
    ] {
        assert_eq!(inspect(path), Err(LocalModelFileRejection::UnsupportedPath));
    }
}

#[cfg(unix)]
#[test]
fn rejects_symlinks_to_files_elsewhere() {
    let directory = tempfile::tempdir().unwrap();
    let target = write(directory.path(), "real.gguf", &chat_model("m", 8_192));
    let link = directory.path().join("link.gguf");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(
        inspect(raw(&link)),
        Err(LocalModelFileRejection::UnsupportedPath)
    );
    assert_eq!(identity(&link), None);
}
