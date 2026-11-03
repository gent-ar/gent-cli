pub(super) fn valid_run(run: &str) -> bool {
    !run.is_empty()
        && run.len() <= 160
        && run
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub(super) fn generation(line: &[u8], profile: &str, run: &str) -> Option<String> {
    if line.len() > 1024 || !line.ends_with(b"\n") {
        return None;
    }
    let line = std::str::from_utf8(line).ok()?.strip_suffix('\n')?;
    let fields = line.split(' ').collect::<Vec<_>>();
    if fields.len() != 4
        || fields[0] != "GENT-SANDBOX/1"
        || fields[2] != profile
        || fields[3] != run
    {
        return None;
    }
    let (pid, created) = fields[1].split_once(':')?;
    if pid.parse::<u32>().ok()? == 0 || created.parse::<u64>().ok()? == 0 {
        return None;
    }
    Some(fields[1].into())
}

pub(super) fn handshake(
    line: &[u8],
    profile: &str,
    run: &str,
    gateway: bool,
) -> Option<(String, Option<crate::SandboxNetworkPeer>)> {
    if !gateway {
        return generation(line, profile, run).map(|id| (id, None));
    }
    if line.len() > 1024 {
        return None;
    }
    let text = std::str::from_utf8(line).ok()?.strip_suffix('\n')?;
    let fields = text.split(' ').collect::<Vec<_>>();
    if fields.len() != 5 {
        return None;
    }
    let base = format!("{} {} {} {}\n", fields[0], fields[1], fields[2], fields[3]);
    let id = generation(base.as_bytes(), profile, run)?;
    let peer = fields[4]
        .strip_prefix("package=")?
        .split(',')
        .collect::<Vec<_>>();
    if peer.len() != 6 {
        return None;
    }
    let peer = crate::SandboxNetworkPeer::external_package(
        peer[0].into(),
        peer[1].into(),
        peer[2].into(),
        peer[3].parse().ok()?,
        peer[4].parse().ok()?,
        peer[5].parse().ok()?,
    )?;
    // The external proxy cannot be the provider process pretending to attest itself.
    if id.split_once(':')?.0.parse::<u32>().ok()? == peer.process_id() {
        return None;
    }
    Some((id, Some(peer)))
}
