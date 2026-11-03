use super::*;
use serde_json::json;
use std::path::Path;

#[test]
fn exact_injected_selected_catalog_tools_allow_both_sdk_spellings_only() {
    let expected =
        crate::standalone_mcp_config::InjectedInternalMcp::fixture(Path::new("/data"), "/bin/gent")
            .servers()
            .clone();
    let actual = crate::conversation_scoped_mcp::scope_server_map(&expected, "conversation");
    for server in gent_types::internal_tools::INTERNAL_SERVER_NAMES {
        for tool in gent_types::internal_tools::tools(server.strip_prefix("gent-")) {
            for spelling in [server.to_owned(), server.replace('-', "_")] {
                assert!(permits_expected(
                    &actual,
                    &expected,
                    "conversation",
                    &format!("mcp__{spelling}__{}", tool["name"].as_str().unwrap()),
                    &[]
                ));
            }
        }
    }
    for unknown in ["mcp__gent_goal__unknown", "mcp__gent-goal__gent_chat_list"] {
        assert!(!permits_expected(
            &actual,
            &expected,
            "conversation",
            unknown,
            &[]
        ));
    }
}

#[test]
fn forged_excluded_unselected_and_cross_conversation_maps_never_mint_trust() {
    let expected =
        crate::standalone_mcp_config::InjectedInternalMcp::fixture(Path::new("/data"), "/bin/gent")
            .servers()
            .clone();
    let actual = crate::conversation_scoped_mcp::scope_server_map(&expected, "conversation");
    for mutation in 0..7 {
        let mut forged = actual.clone();
        match mutation {
            0 => forged["gent-chat"]["command"] = json!("evil"),
            1 => forged["gent-chat"]["env"] = json!({"EXTRA":"value"}),
            2 => forged["gent-chat"]["args"][1] = json!("/other-data"),
            3 => forged["gent-chat"]["args"][5] = json!("other-conversation"),
            4 => {
                forged.as_object_mut().unwrap().remove("gent-chat");
            }
            5 => {
                forged["gent_chat"] = forged["gent-chat"].clone();
            }
            _ => forged["gent-chat"]["args"] = json!(["mcp", "chat"]),
        }
        assert!(
            !permits_expected(
                &forged,
                &expected,
                "conversation",
                "mcp__gent_chat__gent_chat_list",
                &[]
            ),
            "{mutation}"
        );
    }
    for rule in [
        "mcp__gent-chat__*",
        "mcp__gent_chat__gent_chat_list",
        "mcp__gent*__gent_chat_*",
    ] {
        assert!(!permits_expected(
            &actual,
            &expected,
            "conversation",
            "mcp__gent_chat__gent_chat_list",
            &[rule.into()]
        ));
    }
    assert!(!permits(
        None,
        None,
        "conversation",
        "mcp__gent_goal__gent_goal_update",
        &[]
    ));
}

#[test]
fn external_sdk_aliases_are_rejected_but_old_canonical_config_can_be_loaded() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("mcp.json");
    for server in gent_types::internal_tools::INTERNAL_SERVER_NAMES {
        let config = json!({"mcpServers":{(server.replace('-',"_")):{"command":"external"}}});
        std::fs::write(&path, config.to_string()).unwrap();
        assert!(crate::standalone_mcp_config::StandaloneMcpConfig::load(&path).is_err());
        let config = json!({"mcpServers":{(server):{"command":"previous-gent"}}});
        std::fs::write(&path, config.to_string()).unwrap();
        assert!(crate::standalone_mcp_config::StandaloneMcpConfig::load(&path).is_ok());
    }
}

#[test]
fn only_the_exact_minted_descriptor_can_authorize_the_selected_map() {
    let one = crate::standalone_mcp_config::InjectedInternalMcp::fixture(
        Path::new("/data"),
        "one-exact-cli",
    );
    let selected = crate::conversation_scoped_mcp::scope_server_map(one.servers(), "conversation");
    let other_cli = crate::standalone_mcp_config::InjectedInternalMcp::fixture(
        Path::new("/data"),
        "another-layout-candidate",
    );
    let other_root = crate::standalone_mcp_config::InjectedInternalMcp::fixture(
        Path::new("/other"),
        "one-exact-cli",
    );
    let name = "mcp__gent_goal__gent_goal_update";
    assert!(permits(
        Some(&selected),
        Some(&one),
        "conversation",
        name,
        &[]
    ));
    for absent_or_other in [None, Some(&other_cli), Some(&other_root)] {
        assert!(!permits(
            Some(&selected),
            absent_or_other,
            "conversation",
            name,
            &[]
        ));
    }
}

#[test]
fn actual_injection_overwrites_old_canonical_entries_and_reload_cannot_mint_provenance() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("input.json");
    let old = serde_json::json!({"mcpServers":{"gent-goal":{"command":"old-or-external"}}});
    std::fs::write(&path, old.to_string()).unwrap();
    let raw = crate::standalone_mcp_config::StandaloneMcpConfig::load(&path).unwrap();
    assert!(raw.injection().is_none());
    let injected = raw
        .with_fixture_internal_servers(root.path(), "one-exact-cli")
        .unwrap();
    let authority = injected.injection().unwrap();
    let actual = crate::conversation_scoped_mcp::scope_server_map(
        &injected.codex_servers().unwrap(),
        "conversation",
    );
    assert!(permits(
        Some(&actual),
        Some(&authority),
        "conversation",
        "mcp__gent_goal__gent_goal_update",
        &[]
    ));
    assert_eq!(actual["gent-goal"]["command"], "one-exact-cli");
    let reload = crate::standalone_mcp_config::StandaloneMcpConfig::load(injected.path()).unwrap();
    assert!(
        reload.injection().is_none(),
        "serialized configuration is not injection authority"
    );
}
