use super::*;
use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, Instant};
#[test]
#[ignore = "requires exact packaged worker/helper, finite OS closure and official Qwen fixture"]
fn real_lease_authenticated_model_json_sse_revoke_and_unknown() {
    let root = PathBuf::from(std::env::var_os("GENT_QA_MODEL_WORKING_ROOT").unwrap());
    std::fs::create_dir(&root).unwrap();
    let positive = root.join("gateway");
    std::fs::create_dir(&positive).unwrap();
    let (model, released, ledger, sdk, run) =
        crate::contained_model::native_tests::gateway_fixture(&positive);
    let model_root = model.test_root();
    let lease = ProviderGatewayLease::claurst_contained(
        ledger.clone(),
        &binding(&ledger, &run),
        sdk.clone(),
        &positive,
        model.clone(),
    )
    .unwrap();
    assert!(!lease.mcp_settled());
    assert!(
        ProviderGatewayLease::claurst_contained(
            ledger.clone(),
            &binding(&ledger, "another-run"),
            sdk.clone(),
            &positive,
            model.clone()
        )
        .is_err()
    );
    let mut changed_sdk = sdk.clone();
    changed_sdk.digest_sha256 = "0".repeat(64);
    assert!(
        ProviderGatewayLease::claurst_contained(
            lease.fence.ledger.clone(),
            &binding(&lease.fence.ledger, &run),
            changed_sdk,
            &positive,
            model.clone()
        )
        .is_err()
    );
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let url = format!("{}/chat/completions", lease.broker.client_base_url());
    let body = serde_json::json!({"model":"gent-contained-test","messages":[{"role":"user","content":"Say hello."}],"temperature":0,"max_tokens":64,"stream":false,"chat_template_kwargs":{"enable_thinking":false}});
    assert_eq!(
        client
            .post(&url)
            .bearer_auth("owned-wrong-token")
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&body).unwrap())
            .send()
            .unwrap()
            .status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    assert!(model.current());
    let response = client
        .post(&url)
        .bearer_auth(lease.broker.client_token())
        .header("content-type", "application/json")
        .body(serde_json::to_vec(&body).unwrap())
        .send()
        .unwrap();
    assert!(response.status().is_success());
    let value: serde_json::Value = serde_json::from_slice(&response.bytes().unwrap()).unwrap();
    assert!(value["choices"][0]["message"]["content"].is_string());
    assert!(value["usage"]["prompt_tokens"].as_u64().unwrap() > 0);
    let mut stream_body = body.clone();
    stream_body["stream"] = serde_json::json!(true);
    let mut response = client
        .post(&url)
        .bearer_auth(lease.broker.client_token())
        .header("content-type", "application/json")
        .body(serde_json::to_vec(&stream_body).unwrap())
        .send()
        .unwrap();
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut stream = String::new();
    response.read_to_string(&mut stream).unwrap();
    assert!(stream.contains("data: [DONE]"));
    assert!(model.json(serde_json::to_vec(&body).unwrap()).unwrap()["choices"].is_array());
    lease.revoke();
    assert!(lease.mcp_settled() && !lease.current());
    drop(lease);
    drop(model);
    let until = Instant::now() + Duration::from_secs(3);
    while model_root.exists() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!model_root.exists() && released.load(Ordering::Acquire));
    let unknown = root.join("unknown");
    std::fs::create_dir(&unknown).unwrap();
    let (model, released, ledger, sdk, run) =
        crate::contained_model::native_tests::gateway_fixture(&unknown);
    let held_root = model.test_root();
    let lease = ProviderGatewayLease::claurst_contained(
        ledger.clone(),
        &binding(&ledger, &run),
        sdk,
        &unknown,
        model.clone(),
    )
    .unwrap();
    model.test_force_helper_kill();
    lease.revoke();
    assert!(!lease.mcp_settled() && !lease.current());
    super::super::bound_process::retain_unsettled(&lease);
    drop(model);
    drop(lease);
    assert!(held_root.exists() && !released.load(Ordering::Acquire));
    assert!(std::fs::remove_file(held_root.join("launch.json")).is_err());
    std::fs::write(std::env::var_os("GENT_QA_MODEL_REPORT").unwrap(), serde_json::to_vec_pretty(&serde_json::json!({
        "passed":true,"actualProviderGatewayLease":true,"wrongBearerDenied":true,"wrongRunDenied":true,"json":true,"sse":true,"daemonSummarySameActor":true,"knownReleasesRoot":true,"unknownRetainsLeaseRootAndPins":true,"providerSdkStarted":false
    })).unwrap()).unwrap();
}

fn binding(ledger: &gent_store::SqliteLedger, run: &str) -> gent_types::SandboxPermissionBinding {
    let policy = gent_ports::PolicyLedger::current_policy(
        ledger,
        "workspace",
        gent_types::PolicyScope::ProviderPermissions,
    )
    .unwrap()
    .unwrap();
    gent_types::SandboxPermissionBinding::new(
        "conversation".into(),
        run.into(),
        policy,
        gent_types::HostEpoch(1),
    )
    .unwrap()
}
