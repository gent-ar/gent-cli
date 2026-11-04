pub(crate) struct WarmRun<'a> {
    pub(crate) run_id: &'a str,
    pub(crate) conversation_id: &'a str,
    pub(crate) idle: bool,
    pub(crate) last_dispatch: u64,
}

pub(crate) fn evictions<'a>(
    runs: impl IntoIterator<Item = WarmRun<'a>>,
    run_id: &str,
    conversation_id: &str,
    capacity: usize,
) -> Vec<String> {
    let runs = runs.into_iter().collect::<Vec<_>>();
    let (superseded, kept): (Vec<_>, Vec<_>) = runs.into_iter().partition(|run| {
        run.idle && run.conversation_id == conversation_id && run.run_id != run_id
    });
    let mut idle = kept.iter().filter(|run| run.idle).collect::<Vec<_>>();
    idle.sort_by_key(|run| run.last_dispatch);
    let overflow = kept.len().saturating_add(1).saturating_sub(capacity.max(1));
    superseded
        .iter()
        .chain(idle.into_iter().take(overflow))
        .map(|run| run.run_id.to_owned())
        .collect()
}

#[cfg(test)]
#[path = "provider_warm_pool_tests.rs"]
mod tests;
