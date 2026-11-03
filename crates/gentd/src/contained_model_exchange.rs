use super::budget::ResponseBudget;
use super::{Event, Job, Metadata, ModelError, Reader, Scope};
use std::time::Instant;

pub(super) fn run(scope: &Scope, reader: &mut Reader, job: &Job) -> Result<(), ModelError> {
    let started = Instant::now();
    let mut frames = 0usize;
    let mut total = 0usize;
    let result = (|| {
        if !scope.current() {
            return Err(ModelError::Revoked);
        }
        scope.write(&job.encoded)?;
        trace("write_complete", started, frames, total, None);
        let mut budget = ResponseBudget::new(Instant::now());
        let metadata = Metadata::parse(&frame(reader, &budget)?)?;
        if !budget.progress(Instant::now()) {
            return Err(budget.expired_kind(Instant::now()));
        }
        trace("metadata_valid", started, frames, total, None);
        job.events
            .send(Event::Metadata(metadata))
            .map_err(|_| ModelError::Transport)?;
        loop {
            let data = frame(reader, &budget)?;
            if !budget.progress(Instant::now()) {
                return Err(budget.expired_kind(Instant::now()));
            }
            if !scope.current() {
                return Err(ModelError::Revoked);
            }
            if data.is_empty() {
                job.events
                    .send(Event::End)
                    .map_err(|_| ModelError::Transport)?;
                return Ok(());
            }
            total += data.len();
            frames += 1;
            if total > 4 * 1024 * 1024 {
                return Err(ModelError::Protocol);
            }
            if frames == 1 || frames.is_multiple_of(64) {
                trace("data_progress", started, frames, total, None);
            }
            job.events
                .send(Event::Data(data))
                .map_err(|_| ModelError::Transport)?;
        }
    })();
    trace(
        if result.is_ok() { "complete" } else { "error" },
        started,
        frames,
        total,
        result.err(),
    );
    result
}
fn frame(reader: &mut Reader, budget: &ResponseBudget) -> Result<Vec<u8>, ModelError> {
    reader.frame(budget.deadline()).map_err(|error| {
        if error == ModelError::Timeout {
            budget.expired_kind(Instant::now())
        } else {
            error
        }
    })
}
// Test-only owned diagnostics contain numbers and static categories, not data.
#[cfg(test)]
fn trace(stage: &str, started: Instant, frames: usize, bytes: usize, error: Option<ModelError>) {
    use std::io::Write;
    if let Some(path) = std::env::var_os("GENT_FACTORY_MODEL_TRACE") {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            if f.metadata().is_ok_and(|m| m.len() < 16384) {
                let _ = writeln!(
                    f,
                    "stage={stage} elapsed_ms={} frames={frames} bytes={bytes} error={error:?}",
                    started.elapsed().as_millis()
                );
            }
        }
    }
}
#[cfg(not(test))]
fn trace(_: &str, _: Instant, _: usize, _: usize, _: Option<ModelError>) {}
