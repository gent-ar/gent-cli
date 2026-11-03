use super::*;

fn policy() -> serde_json::Value {
    serde_json::json!({
        "schema":1,"policy":"external-identity-proxy-private-v1","provider":"codex",
        "ingress":"private-network-capability-accepted","physicalLanVerified":false,
        "helperSha256":"a".repeat(64),"resourcesSha256":"b".repeat(64),
        "package":{"family":"Gent.Proxy_abc","fullName":"Gent.Proxy_1.0.0.0_x64__abc",
            "application":"Proxy","imageSha256":"c".repeat(64),
            "manifestSha256":"d".repeat(64),"developmentRegistration":true}
    })
}

fn decode(value: serde_json::Value) -> Result<ExternalProxyPolicy, String> {
    ExternalProxyPolicy::decode(&serde_json::to_vec(&value).unwrap())
}

#[test]
fn exact_scope_and_artifacts_are_required_without_claiming_lan_proof() {
    let p = decode(policy()).unwrap();
    assert!(!p.physical_lan_verified);
    assert!(
        p.require(
            AgentChatProvider::Codex,
            &"a".repeat(64),
            Some(&"b".repeat(64))
        )
        .is_ok()
    );
    assert!(
        p.require(
            AgentChatProvider::Claude,
            &"a".repeat(64),
            Some(&"b".repeat(64))
        )
        .is_err()
    );
    assert!(
        p.require(
            AgentChatProvider::Claurst,
            &"a".repeat(64),
            Some(&"b".repeat(64))
        )
        .is_err()
    );
    assert!(
        p.require(
            AgentChatProvider::Codex,
            &"c".repeat(64),
            Some(&"b".repeat(64))
        )
        .is_err()
    );
    assert!(
        p.require(AgentChatProvider::Codex, &"a".repeat(64), None)
            .is_err()
    );
    assert!(
        p.require(
            AgentChatProvider::Codex,
            &"a".repeat(64),
            Some(&"c".repeat(64))
        )
        .is_err()
    );
}

#[test]
fn complete_or_denied_ingress_cannot_be_substituted_for_the_accepted_scope() {
    for (field, value) in [
        ("physicalLanVerified", serde_json::json!(true)),
        ("ingress", serde_json::json!("deny-all")),
        ("policy", serde_json::json!("complete-network-policy")),
        ("schema", serde_json::json!(2)),
    ] {
        let mut p = policy();
        p[field] = value;
        assert!(decode(p).is_err());
    }
}

#[test]
fn malformed_and_unknown_descriptor_fields_fail_closed() {
    assert!(ExternalProxyPolicy::decode(&vec![b'x'; 16 * 1024 + 1]).is_err());
    let mut p = policy();
    p["skipChecks"] = true.into();
    assert!(decode(p).is_err());
    let mut p = policy();
    p["package"]["family"] = "../outside".into();
    assert!(decode(p).is_err());
    let mut p = policy();
    p["helperSha256"] = "A".repeat(64).into();
    assert!(decode(p).is_err());
}

#[test]
fn real_package_tuple_including_development_status_is_exact() {
    let p = decode(policy()).unwrap();
    assert!(p.require_package(&p.package).is_ok());
    let mut other = p.package.clone();
    other.image_sha256 = "e".repeat(64);
    assert!(p.require_package(&other).is_err());
    let mut other = p.package.clone();
    other.development_registration = false;
    assert!(p.require_package(&other).is_err());
    let mut other = p.package.clone();
    other.full_name = "Gent.Proxy_2.0.0.0_x64__abc".into();
    assert!(p.require_package(&other).is_err());
}

#[test]
fn claude_authority_never_admits_codex_or_claurst() {
    let mut value = policy();
    value["provider"] = "claude".into();
    let p = decode(value).unwrap();
    assert!(
        p.require(
            AgentChatProvider::Claude,
            &"a".repeat(64),
            Some(&"b".repeat(64))
        )
        .is_ok()
    );
    for provider in [AgentChatProvider::Codex, AgentChatProvider::Claurst] {
        assert!(
            p.require(provider, &"a".repeat(64), Some(&"b".repeat(64)))
                .is_err()
        );
    }
}

#[test]
fn schema_two_binds_each_explicit_reviewed_provider_to_the_same_artifacts() {
    let mut value = policy();
    value.as_object_mut().unwrap().remove("provider");
    value["schema"] = 2.into();
    value["providers"] = serde_json::json!(["codex", "claude"]);
    let descriptor = decode(value.clone()).unwrap();
    for selected in [AgentChatProvider::Codex, AgentChatProvider::Claude] {
        assert!(
            descriptor
                .require(selected, &"a".repeat(64), Some(&"b".repeat(64)))
                .is_ok()
        );
        assert!(
            descriptor
                .require(selected, &"c".repeat(64), Some(&"b".repeat(64)))
                .is_err()
        );
        assert!(descriptor.require(selected, &"a".repeat(64), None).is_err());
    }
    assert!(
        descriptor
            .require(
                AgentChatProvider::Claurst,
                &"a".repeat(64),
                Some(&"b".repeat(64))
            )
            .is_err()
    );
    value["providers"] = serde_json::json!(["claude"]);
    let descriptor = decode(value).unwrap();
    assert!(
        descriptor
            .require(
                AgentChatProvider::Codex,
                &"a".repeat(64),
                Some(&"b".repeat(64))
            )
            .is_err()
    );
}

#[test]
fn schema_two_rejects_ambiguous_duplicate_empty_and_unreviewed_provider_scope() {
    for providers in [
        serde_json::json!([]),
        serde_json::json!(["codex", "codex"]),
        serde_json::json!(["unknown"]),
        serde_json::json!(["codex", "claude", "codex"]),
    ] {
        let mut value = policy();
        value.as_object_mut().unwrap().remove("provider");
        value["schema"] = 2.into();
        value["providers"] = providers;
        assert!(decode(value).is_err());
    }
    for legacy in [serde_json::json!("codex"), serde_json::Value::Null] {
        let mut value = policy();
        value["schema"] = 2.into();
        value["provider"] = legacy;
        value["providers"] = serde_json::json!(["claude"]);
        assert!(decode(value).is_err());
    }
    for modern in [serde_json::json!([]), serde_json::Value::Null] {
        let mut value = policy();
        value["providers"] = modern;
        assert!(decode(value).is_err());
    }
}

#[test]
fn claurst_requires_its_own_explicit_exact_private_scope() {
    let mut value = policy();
    value["provider"] = serde_json::json!("claurst");
    let descriptor = decode(value).unwrap();
    assert!(!descriptor.physical_lan_verified);
    assert!(
        descriptor
            .require(
                AgentChatProvider::Claurst,
                &"a".repeat(64),
                Some(&"b".repeat(64))
            )
            .is_ok()
    );
    for provider in [AgentChatProvider::Claude, AgentChatProvider::Codex] {
        assert!(
            descriptor
                .require(provider, &"a".repeat(64), Some(&"b".repeat(64)))
                .is_err()
        );
    }
    assert!(
        descriptor
            .require(
                AgentChatProvider::Claurst,
                &"c".repeat(64),
                Some(&"b".repeat(64))
            )
            .is_err()
    );
    let mut value = policy();
    value.as_object_mut().unwrap().remove("provider");
    value["schema"] = 2.into();
    value["providers"] = serde_json::json!(["codex", "claude", "claurst"]);
    assert!(
        decode(value)
            .unwrap()
            .require(
                AgentChatProvider::Claurst,
                &"a".repeat(64),
                Some(&"b".repeat(64))
            )
            .is_ok()
    );
}
