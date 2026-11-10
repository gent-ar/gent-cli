use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use crate::standalone_authority_composition::{
    StandaloneClaurstModelStatus, StandaloneClaurstModels,
};

const RUNTIME_LIBRARY_PREFIXES: [&str; 3] = ["ggml", "llama", "cublas"];

pub(crate) fn spawn(
    models: StandaloneClaurstModels,
    model_id: String,
    llama_server: PathBuf,
    stop: impl Fn() -> bool + Send + 'static,
) {
    let started = std::thread::Builder::new()
        .name("gent-local-runtime-prewarm".into())
        .spawn(move || {
            if let Some(model_path) = ready_model(&models, &model_id) {
                warm(&files(&model_path, &llama_server), &stop, |path| {
                    File::open(path)
                });
            }
        });
    if let Err(error) = started {
        eprintln!("local runtime prewarm could not start: {error}");
    }
}

pub(crate) fn local_model_id(data_dir: &Path) -> String {
    let selection =
        crate::runtime_facade::model_catalog::defaults::ModelCatalogDefaults::open(data_dir)
            .selection();
    if selection.provider == gent_types::AgentChatProvider::Claurst {
        selection.model
    } else {
        gent_protocol::DEFAULT_LOCAL_MODEL_ID.into()
    }
}

pub(crate) fn ready_model(models: &StandaloneClaurstModels, model_id: &str) -> Option<PathBuf> {
    match models.assess(model_id) {
        Ok(StandaloneClaurstModelStatus::Ready { model_path }) => Some(model_path),
        _ => None,
    }
}

pub(crate) fn files(model_path: &Path, llama_server: &Path) -> Vec<PathBuf> {
    let mut libraries = llama_server
        .parent()
        .and_then(|directory| std::fs::read_dir(directory).ok())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| runtime_library(path))
        .collect::<Vec<_>>();
    libraries.sort();
    std::iter::once(model_path.to_path_buf())
        .chain(libraries)
        .collect()
}

fn runtime_library(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
        && RUNTIME_LIBRARY_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

pub(crate) fn warm<R: Read>(
    files: &[PathBuf],
    stop: &impl Fn() -> bool,
    open: impl Fn(&Path) -> std::io::Result<R>,
) -> u64 {
    let mut buffer = vec![0_u8; 4 * 1024 * 1024];
    let mut total = 0_u64;
    for path in files {
        let Ok(mut reader) = open(path) else {
            continue;
        };
        loop {
            if stop() {
                return total;
            }
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => total += count as u64,
            }
        }
    }
    total
}

#[cfg(test)]
#[path = "local_runtime_prewarm_tests.rs"]
mod tests;
