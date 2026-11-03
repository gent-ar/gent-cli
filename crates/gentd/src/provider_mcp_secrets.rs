//! Bounded matching of explicitly captured credential values, not general DLP.
use serde_json::Value;

pub fn contains(value: &Value, secrets: &[String]) -> bool {
    let matched = |text: &str| secrets.iter().any(|s| !s.is_empty() && text.contains(s));
    match value {
        Value::String(text) => matched(text),
        Value::Array(values) => values.iter().any(|v| contains(v, secrets)),
        Value::Object(values) => values
            .iter()
            .any(|(k, v)| matched(k) || contains(v, secrets)),
        _ => false,
    }
}
fn credential_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "CREDENTIAL",
        "AUTH",
        "COOKIE",
        "BEARER",
        "API_KEY",
        "PRIVATE_KEY",
        "ACCESS_KEY",
        "HEADER",
    ]
    .iter()
    .any(|part| key.contains(part))
}
fn add(output: &mut Vec<String>, value: &str) {
    if value.is_empty() {
        return;
    }
    output.push(value.into());
    if let Ok(value) = serde_json::from_str::<Value>(value) {
        fn strings(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::String(s) => {
                    out.push(s.clone());
                    if let Some((scheme, token)) = s.split_once(' ') {
                        if scheme.eq_ignore_ascii_case("bearer")
                            || scheme.eq_ignore_ascii_case("basic")
                        {
                            out.push(token.to_owned());
                        }
                    }
                }
                Value::Array(a) => {
                    for v in a {
                        strings(v, out)
                    }
                }
                Value::Object(m) => {
                    for v in m.values() {
                        strings(v, out)
                    }
                }
                _ => (),
            }
        }
        strings(&value, output);
    }
    // Authorization/header forms: match the credential itself as well as the
    // original host-side header so a server cannot strip the public prefix.
    let value = value
        .split_once(':')
        .filter(|(name, _)| !name.contains(['/', '\\', ' ']))
        .map_or(value, |(_, v)| v.trim());
    output.push(value.to_owned());
    for pair in value.split(';') {
        if let Some((_, secret)) = pair.split_once('=') {
            if !secret.trim().is_empty() {
                output.push(secret.trim().to_owned());
            }
        }
    }
    if let Some((scheme, token)) = value.split_once(' ') {
        if scheme.eq_ignore_ascii_case("bearer") || scheme.eq_ignore_ascii_case("basic") {
            if !token.is_empty() {
                output.push(token.into());
            }
        }
    }
}
pub fn captured(env: &std::collections::BTreeMap<String, String>, args: &[String]) -> Vec<String> {
    let mut output = Vec::new();
    for (key, value) in env {
        if credential_key(key) {
            add(&mut output, value);
        }
    }
    let mut next = false;
    for arg in args {
        if next {
            add(&mut output, arg);
            next = false;
        }
        if arg.starts_with('-') {
            if let Some((flag, value)) = arg.split_once('=') {
                if credential_key(flag) {
                    add(&mut output, value);
                }
            } else if credential_key(arg) {
                next = true;
            }
            if matches!(arg.as_str(), "--header" | "--headers" | "-H") {
                next = true;
            }
        }
        if let Ok(url) = reqwest::Url::parse(arg) {
            if let Some(password) = url.password() {
                add(&mut output, password);
                let mut decoded = Vec::new();
                let bytes = password.as_bytes();
                let mut i = 0;
                while i < bytes.len() {
                    if bytes[i] == b'%' && i + 2 < bytes.len() {
                        let hex = |b: u8| char::from(b).to_digit(16);
                        if let (Some(a), Some(b)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                            decoded.push(u8::try_from(a * 16 + b).unwrap_or_default());
                            i += 3;
                            continue;
                        }
                    }
                    decoded.push(bytes[i]);
                    i += 1;
                }
                if let Ok(decoded) = String::from_utf8(decoded) {
                    add(&mut output, &decoded);
                }
            }
            for (_, value) in url.query_pairs() {
                add(&mut output, &value);
            }
        }
    }
    output.sort();
    output.dedup();
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn mcp_secret_escaped_quote_backslash_and_unicode_are_matched_after_decode() {
        let secret = "canary-\"quoted\\path-雪".to_string();
        let encoded = serde_json::to_string(&json!({"result":[{"text":secret}]})).unwrap();
        assert!(!encoded.contains(&secret));
        let decoded: Value = serde_json::from_str(&encoded).unwrap();
        assert!(contains(&decoded, std::slice::from_ref(&secret)));
        assert!(contains(&json!({(secret.clone()):"value"}), &[secret]));
    }
    #[test]
    fn mcp_secret_host_env_and_argv_credentials_stay_in_catalog() {
        let env = std::collections::BTreeMap::from([
            ("GENT_BROWSER_TOKEN".into(), "browser-canary".into()),
            ("CUSTOM_CREDENTIAL".into(), "short".into()),
            ("PATH".into(), "public-path".into()),
        ]);
        let values = captured(
            &env,
            &[
                "--auth-token=argument-canary".into(),
                "--password".into(),
                "private-canary".into(),
                "https://user:url-canary@example.invalid".into(),
            ],
        );
        for expected in [
            "browser-canary",
            "short",
            "argument-canary",
            "private-canary",
            "url-canary",
        ] {
            assert!(values.iter().any(|v| v == expected));
        }
        assert!(!values.iter().any(|v| v == "public-path"));
    }
    #[test]
    fn mcp_secret_header_and_query_carriers_capture_opaque_values() {
        let values = captured(
            &std::collections::BTreeMap::new(),
            &[
                "--header".into(),
                "Authorization: Bearer header-canary".into(),
                "https://service.invalid/?token=query%22canary%5Cvalue".into(),
                "https://user:password%22canary@example.invalid".into(),
                "--headers".into(),
                r#"{"Authorization":"Bearer json-header-canary"}"#.into(),
            ],
        );
        for expected in [
            "header-canary",
            "query\"canary\\value",
            "password\"canary",
            "json-header-canary",
        ] {
            assert!(values.iter().any(|v| v == expected));
        }
    }
}
