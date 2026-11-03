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
        .split(['-', '+'])
        .next()
        .unwrap_or_default()
        .split('.')
        .filter_map(|part| part.parse().ok())
        .collect()
}

pub(super) fn valid_release(value: &str, target: &str) -> bool {
    let Some(version) = value.strip_suffix(&format!("-{target}")) else {
        return false;
    };
    if valid_version(version) {
        return true;
    }
    is_generation(value, target)
}

pub(super) fn is_generation(value: &str, target: &str) -> bool {
    let Some(version) = value.strip_suffix(&format!("-{target}")) else {
        return false;
    };
    let Some((core, qualifier)) = version.split_once('+') else {
        return false;
    };
    let Some((digest, nonce)) = qualifier.split_once('.') else {
        return false;
    };
    valid_version(core)
        && digest.len() == 16
        && matches!(nonce.len(), 8 | 64)
        && [digest, nonce].into_iter().all(|value| {
            value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        })
}
