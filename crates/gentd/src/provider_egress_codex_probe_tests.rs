use super::*;
use serde_json::json;

fn config(home: &Path, store: &str) -> Value {
    json!({"jsonrpc":"2.0","id":2,"result":{
        "config":{"cli_auth_credentials_store":store},"layers":[{
            "name":{"type":"user","file":home.join("config.toml"),"profile":null},
            "version":"fixture","config":{},"disabledReason":null
        }]
    }})
}

#[test]
fn account_refresh_follows_supported_native_store_discovery() {
    let home = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let mut probe = CodexCredentialProbe::new(&workspace.path().canonicalize().unwrap());
    let request: Value =
        serde_json::from_slice(&CodexCredentialProbe::configuration_request().unwrap()).unwrap();
    assert_eq!(request["method"], "config/read");
    let Progress::RequestAccount(frame) = probe
        .accept(&config(&home.path().canonicalize().unwrap(), "file"))
        .unwrap()
    else {
        panic!("supported store must request native account metadata")
    };
    let request: Value = serde_json::from_slice(&frame).unwrap();
    assert_eq!(request["method"], "account/read");
    assert_eq!(request["params"]["refreshToken"], true);
    let response = json!({"id":3,"result":{"account":{"type":"apiKey"},"requiresOpenaiAuth":true}});
    assert!(matches!(
        probe.accept(&response).unwrap(),
        Progress::Done(_)
    ));
}

#[test]
fn unsupported_store_or_workspace_overlap_never_requests_account_refresh() {
    let home = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    for store in ["keyring", "auto", "ephemeral"] {
        let mut probe = CodexCredentialProbe::new(&workspace.path().canonicalize().unwrap());
        assert!(
            probe
                .accept(&config(&home.path().canonicalize().unwrap(), store))
                .is_err()
        );
    }
    let home = home.path().canonicalize().unwrap();
    assert!(
        CodexCredentialProbe::new(&home)
            .accept(&config(&home, "file"))
            .is_err()
    );
}

#[test]
fn unrelated_or_out_of_order_native_frames_cannot_bind_credentials() {
    let workspace = tempfile::tempdir().unwrap();
    let mut probe = CodexCredentialProbe::new(&workspace.path().canonicalize().unwrap());
    assert!(matches!(
        probe
            .accept(&json!({"method":"account/updated","params":{}}))
            .unwrap(),
        Progress::Continue
    ));
    assert!(
        probe
            .accept(&json!({"id":3,"result":{"account":{"type":"apiKey"}}}))
            .is_err()
    );
    assert!(
        probe
            .accept(&json!({"id":2,"error":{"message":"fixture"}}))
            .is_err()
    );
}
