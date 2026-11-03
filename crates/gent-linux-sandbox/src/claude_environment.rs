//! Fixed credential-free traffic isolation from Gent's managed Claude probe.
pub const FLAGS: &[(&str, &str)] = &[
    ("DISABLE_AUTOUPDATER", "1"),
    ("DISABLE_TELEMETRY", "1"),
    ("DISABLE_ERROR_REPORTING", "1"),
    ("CLAUDE_CODE_SKIP_PROMPT_HISTORY", "1"),
    ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
    ("CLAUDE_CODE_DISABLE_OFFICIAL_MARKETPLACE_AUTOINSTALL", "1"),
    ("CLAUDE_CODE_DISABLE_POLICY_SKILLS", "1"),
    ("ENABLE_CLAUDEAI_MCP_SERVERS", "false"),
];
pub const MCP_TOKEN_ENV: &str = "GENT_MCP_BRIDGE_TOKEN";
type Environment = Vec<(String, String)>;
pub fn split_private(
    environment: &[(String, String)],
    arguments: &[String],
) -> Result<(Environment, Environment), &'static str> {
    let private_key = |key: &str| key == "ANTHROPIC_AUTH_TOKEN" || key == MCP_TOKEN_ENV;
    let secrets = environment
        .iter()
        .filter(|(key, _)| private_key(key))
        .cloned()
        .collect::<Vec<_>>();
    let mut keys = std::collections::BTreeSet::new();
    if secrets.len() > 2 || secrets.iter().any(|(key, _)| !keys.insert(key)) {
        return Err("duplicate private bearer");
    }
    let public = environment
        .iter()
        .filter(|(key, _)| !private_key(key))
        .cloned()
        .collect::<Vec<_>>();
    if let Some((_, mcp)) = secrets.iter().find(|(key, _)| key == MCP_TOKEN_ENV) {
        if secrets
            .iter()
            .find(|(key, _)| key == "ANTHROPIC_AUTH_TOKEN")
            .map(|(_, token)| token)
            != Some(mcp)
        {
            return Err("MCP bearer alias changed");
        }
    }
    for (_, token) in &secrets {
        if token.len() != 64
            || !token
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("private bearer format");
        }
        if public
            .iter()
            .any(|(key, value)| key.contains(token) || value.contains(token))
            || arguments.iter().any(|value| value.contains(token))
        {
            return Err("private bearer in launch arguments");
        }
    }
    Ok((public, secrets))
}
#[cfg(test)]
mod tests {
    #[test]
    fn bearer_never_enters_sdk_environment_or_provider_arguments() {
        let token = "a".repeat(64);
        let environment = vec![
            ("HOME".into(), "/private/home".into()),
            ("ANTHROPIC_AUTH_TOKEN".into(), token.clone()),
        ];
        let (public, private) = super::split_private(&environment, &["--print".into()]).unwrap();
        assert_eq!(public, vec![("HOME".into(), "/private/home".into())]);
        assert_eq!(
            private,
            vec![("ANTHROPIC_AUTH_TOKEN".into(), token.clone())]
        );
        assert!(super::split_private(&environment, std::slice::from_ref(&token)).is_err());
        let mut duplicated = environment.clone();
        duplicated.push(("ANTHROPIC_AUTH_TOKEN".into(), token.clone()));
        assert!(super::split_private(&duplicated, &[]).is_err());
        let mut hidden = environment;
        hidden.push(("PUBLIC_SETTING".into(), token));
        assert!(super::split_private(&hidden, &[]).is_err());
    }
}

#[cfg(test)]
mod mcp_alias_tests {
    #[test]
    fn mcp_alias_is_same_private_scoped_bearer_and_never_a_public_value() {
        let token = "a".repeat(64);
        let env = vec![
            ("ANTHROPIC_AUTH_TOKEN".into(), token.clone()),
            (super::MCP_TOKEN_ENV.into(), token.clone()),
        ];
        let (public, private) = super::split_private(&env, &[]).unwrap();
        assert!(public.is_empty());
        assert_eq!(private.len(), 2);
        let mut different = env.clone();
        different[1].1 = "b".repeat(64);
        assert!(super::split_private(&different, &[]).is_err());
        assert!(super::split_private(&env, std::slice::from_ref(&token)).is_err());
        let mut leaked = env;
        leaked.push(("PUBLIC".into(), token));
        assert!(super::split_private(&leaked, &[]).is_err());
    }
}
