pub(super) fn valid_version(value: &str) -> bool {
    value.strip_prefix('v').is_some_and(|value| {
        value.split('.').count() == 3
            && value
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    })
}

pub(super) fn version_of(value: &str) -> Vec<u32> {
    value
        .trim_start_matches('v')
        .split('-')
        .next()
        .unwrap_or_default()
        .split('.')
        .filter_map(|part| part.parse().ok())
        .collect()
}
