use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{display, stage};

#[derive(Deserialize, Serialize)]
struct Record {
    schema: u32,
    generation: String,
    bootstrap: PathBuf,
    bootstrap_stat: Vec<stage::StatEntry>,
    release_stat: Vec<stage::StatEntry>,
    tree: String,
    executable: String,
}

pub(super) struct Known {
    pub(super) tree: String,
    pub(super) executable: String,
}

fn path(root: &Path, generation: &str) -> PathBuf {
    root.join("activation-records")
        .join(format!("{generation}.json"))
}

fn read(root: &Path, generation: &str) -> Option<Record> {
    let bytes = fs::read(path(root, generation)).ok()?;
    let record: Record = serde_json::from_slice(&bytes).ok()?;
    (record.schema == 1 && record.generation == generation).then_some(record)
}

fn release_current(root: &Path, record: &Record) -> bool {
    stage::stat(&root.join("releases").join(&record.generation))
        .is_ok_and(|stat| stat == record.release_stat)
}

pub(super) fn unchanged(
    root: &Path,
    generation: &str,
    bootstrap: &Path,
    bootstrap_stat: &[stage::StatEntry],
) -> Option<Known> {
    let record = read(root, generation)?;
    (record.bootstrap == bootstrap
        && record.bootstrap_stat == bootstrap_stat
        && release_current(root, &record))
    .then_some(Known {
        tree: record.tree,
        executable: record.executable,
    })
}

pub(super) fn release(root: &Path, generation: &str) -> Option<Known> {
    let record = read(root, generation)?;
    release_current(root, &record).then_some(Known {
        tree: record.tree,
        executable: record.executable,
    })
}

pub(super) fn write(
    root: &Path,
    generation: &str,
    bootstrap: &Path,
    bootstrap_stat: Vec<stage::StatEntry>,
    known: &Known,
) -> Result<(), String> {
    let record = Record {
        schema: 1,
        generation: generation.into(),
        bootstrap: bootstrap.into(),
        bootstrap_stat,
        release_stat: stage::stat(&root.join("releases").join(generation))?,
        tree: known.tree.clone(),
        executable: known.executable.clone(),
    };
    let directory = root.join("activation-records");
    fs::create_dir_all(&directory).map_err(display)?;
    let mut temporary = tempfile::NamedTempFile::new_in(&directory).map_err(display)?;
    temporary
        .write_all(&serde_json::to_vec(&record).map_err(display)?)
        .map_err(display)?;
    temporary.as_file().sync_all().map_err(display)?;
    temporary
        .persist(path(root, generation))
        .map_err(|error| display(error.error))?;
    Ok(())
}
