use super::*;
use serde_json::json;

fn context(store: Option<&str>, kind: &str) -> (tempfile::TempDir, Value, Value) {
    let root = tempfile::tempdir().unwrap();
    let mut config = json!({"config":{"model_provider":"openai"}, "layers":[{
        "name":{"type":"user","file":root.path().join("config.toml"),"profile":null},
        "version":"fixture", "config":{}, "disabledReason":null
    }]});
    if let Some(store) = store {
        config["config"]["cli_auth_credentials_store"] = store.into();
    }
    let account = json!({"account":{"type":kind,"email":"owner@example.invalid","planType":"plus"}, "requiresOpenaiAuth":true});
    (root, config, account)
}

fn jwt(claims: Value) -> String {
    format!(
        "e30.{}.fixture",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    )
}

fn tokens(expiry: u64) -> Value {
    json!({"auth_mode":"chatgpt", "OPENAI_API_KEY":null, "tokens":{
        "access_token":jwt(json!({"exp":expiry})), "refresh_token":"refresh-never-forward",
        "id_token":jwt(json!({"email":"owner@example.invalid", "https://api.openai.com/auth":{"chatgpt_account_id":"workspace-fixture"}})),
        "account_id":"workspace-fixture"
    }})
}

#[test]
fn pinned_file_store_metadata_and_owner_tokens_produce_only_broker_headers() {
    let (_root, config, account) = context(None, "chatgpt");
    let context = CodexOwnerContext::parse(&config, &account).unwrap();
    let auth = tokens(10_000);
    let credential = context
        .credential(&serde_json::to_vec(&auth).unwrap(), 100)
        .unwrap();
    assert_eq!(
        credential.upstream,
        "https://chatgpt.com/backend-api/codex/"
    );
    assert_eq!(
        credential
            .headers
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["authorization", "chatgpt-account-id"]
    );
    let debug = format!("{credential:?}");
    assert!(!debug.contains(auth["tokens"]["access_token"].as_str().unwrap()));
    assert!(!debug.contains("refresh-never-forward"));
    assert!(
        !credential
            .headers
            .iter()
            .any(|(_, value)| value.contains("refresh-never-forward"))
    );
}

#[test]
fn nonfile_and_unprovable_native_account_contexts_fail_closed() {
    for store in ["auto", "keyring", "ephemeral", "future-store"] {
        let (_root, config, account) = context(Some(store), "chatgpt");
        assert!(CodexOwnerContext::parse(&config, &account).is_err());
    }
    let (_root, mut config, account) = context(Some("file"), "chatgpt");
    config["config"]["model_provider"] = "unreviewed-upstream".into();
    assert!(CodexOwnerContext::parse(&config, &account).is_err());
    let (_root, mut config, account) = context(Some("file"), "chatgpt");
    config["layers"] = json!([]);
    assert!(CodexOwnerContext::parse(&config, &account).is_err());
}

#[test]
fn expired_mismatched_or_unreviewed_credentials_cannot_reach_the_gateway() {
    let (_root, mut config, account) = context(Some("file"), "chatgpt");
    config["config"]["forced_chatgpt_workspace_id"] = "workspace-fixture".into();
    let context = CodexOwnerContext::parse(&config, &account).unwrap();
    assert!(
        context
            .credential(&serde_json::to_vec(&tokens(159)).unwrap(), 100)
            .is_err()
    );
    let mut auth = tokens(10_000);
    auth["tokens"]["account_id"] = "other-workspace".into();
    assert!(
        context
            .credential(&serde_json::to_vec(&auth).unwrap(), 100)
            .is_err()
    );
    let mut auth = tokens(10_000);
    auth["tokens"]["id_token"] = jwt(json!({"email":"other@example.invalid"})).into();
    assert!(
        context
            .credential(&serde_json::to_vec(&auth).unwrap(), 100)
            .is_err()
    );
    let mut auth = tokens(10_000);
    auth["auth_mode"] = "chatgptAuthTokens".into();
    assert!(
        context
            .credential(&serde_json::to_vec(&auth).unwrap(), 100)
            .is_err()
    );
}

#[test]
fn api_key_store_uses_only_fixed_openai_origin_and_redacted_owner_memory() {
    let (_root, config, account) = context(Some("file"), "apiKey");
    let context = CodexOwnerContext::parse(&config, &account).unwrap();
    let auth = json!({"auth_mode":"apikey","OPENAI_API_KEY":"owner-only-api-key-fixture"});
    let credential = context
        .credential(&serde_json::to_vec(&auth).unwrap(), 100)
        .unwrap();
    assert_eq!(credential.upstream, "https://api.openai.com/v1/");
    assert_eq!(credential.headers.len(), 1);
    assert!(!format!("{credential:?}").contains("owner-only-api-key-fixture"));
    assert!(
        context
            .credential(
                br#"{"auth_mode":"apikey","OPENAI_API_KEY":"bad\r\nkey"}"#,
                100
            )
            .is_err()
    );
}

#[test]
fn native_reported_default_chatgpt_base_keeps_the_fixed_broker_authority() {
    for base in [
        "https://chatgpt.com/backend-api",
        "https://chatgpt.com/backend-api/",
    ] {
        let (_root, mut config, account) = context(Some("file"), "chatgpt");
        config["config"]["model_provider"] = Value::Null;
        config["config"]["chatgpt_base_url"] = base.into();
        let owner = CodexOwnerContext::parse(&config, &account).unwrap();
        let credential = owner
            .credential(&serde_json::to_vec(&tokens(10_000)).unwrap(), 100)
            .unwrap();
        assert_eq!(
            credential.upstream,
            "https://chatgpt.com/backend-api/codex/"
        );
        assert_eq!(
            credential
                .headers
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["authorization", "chatgpt-account-id"]
        );
    }
}
#[test]
fn custom_or_ambiguous_chatgpt_base_metadata_cannot_admit_credentials() {
    for base in [
        "http://chatgpt.com/backend-api/",
        "https://chatgpt.com.evil.invalid/backend-api/",
        "https://evil.invalid/backend-api/",
        "https://user:fixture@chatgpt.com/backend-api/",
        "https://chatgpt.com:443/backend-api/",
        "https://chatgpt.com/backend-api/?key=fixture",
        "https://chatgpt.com/backend-api/#fragment",
        "https://chatgpt.com/backend-api/../backend-api/",
        "https://chatgpt.com/backend-api//",
        "",
    ] {
        let (_root, mut config, account) = context(Some("file"), "chatgpt");
        config["config"]["chatgpt_base_url"] = base.into();
        assert!(CodexOwnerContext::parse(&config, &account).is_err());
    }
    for value in [json!(false), json!(7), json!([]), json!({})] {
        let (_root, mut config, account) = context(Some("file"), "chatgpt");
        config["config"]["chatgpt_base_url"] = value;
        assert!(CodexOwnerContext::parse(&config, &account).is_err());
    }
}
