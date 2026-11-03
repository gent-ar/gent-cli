use super::*;
use serde_json::{Value, json};

fn manifest() -> Value {
    json!({"version":1,"providers":[{
        "provider":"claude","platform":"win32-x64", "packageName":"@anthropic-ai/claude-code-win32-x64",
        "packageVersion":"2.1.291","packageIntegrity":"sha512-fixture", "nodeSha256":"a".repeat(64),
        "credentialContract":"claude-loopback-v1", "osLibraries":["bcrypt.dll"],
        "files":[
            {"root":"nodeRuntime","path":"bin/node.exe","sha256":"a".repeat(64)},
            {"root":"nodeRuntime","path":"bin/npm.cmd","sha256":"b".repeat(64)},
            {"root":"nodeRuntime","path":"lib/node_modules/npm/bin/npm-cli.js","sha256":"c".repeat(64)},
            {"root":"gitRuntime","path":"usr/bin/bash.exe","sha256":"d".repeat(64)}
        ]
    }]})
}

fn parse_value(value: &Value) -> Result<ProviderResources, String> {
    parse(&serde_json::to_vec(value).unwrap(), "claude")
}

#[test]
fn git_mount_contract_rejects_missing_or_unreviewed_configuration() {
    let mut contract = parse_value(&manifest()).unwrap();
    assert!(require_git_mounts(&contract).is_err());
    contract.files.push(ResourceFile {
        root: ResourceRoot::GitRuntime,
        path: "etc/fstab".into(),
        sha256: "a".repeat(64),
    });
    assert!(require_git_mounts(&contract).is_err());
    contract.files.last_mut().unwrap().sha256 =
        "af2184544d99dd656f4e5e119d80f2cb15f03c29d3d8467da1df5aa330508f4a".into();
    assert!(require_git_mounts(&contract).is_ok());
}

#[test]
fn reviewed_manifest_describes_only_finite_exact_resources() {
    let contract = parse_value(&manifest()).unwrap();
    assert_eq!(contract.files.len(), 4);
    assert_eq!(contract.os_libraries, ["bcrypt.dll"]);
}

#[test]
fn powershell_leaf_manifest_is_claude_or_codex_only_and_bounded() {
    let mut value = manifest();
    value["providers"][0]["provider"] = "codex".into();
    value["providers"][0]["credentialContract"] = "codex-file-responses-v1".into();
    value["providers"][0]["files"] =
        json!((0..658).map(|n| json!({
        "root":"powerShellRuntime", "path":format!("file-{n}.dll"), "sha256":"a".repeat(64)
    })).collect::<Vec<_>>());
    assert_eq!(
        parse(&serde_json::to_vec(&value).unwrap(), "codex")
            .unwrap()
            .files
            .len(),
        658
    );
    let mut claude = value.clone();
    claude["providers"][0]["provider"] = "claude".into();
    claude["providers"][0]["credentialContract"] = "claude-loopback-v1".into();
    assert_eq!(parse_value(&claude).unwrap().files.len(), 658);
    let file = json!({"root":"powerShellRuntime", "path":"extra.dll", "sha256":"a".repeat(64)});
    value["providers"][0]["files"]
        .as_array_mut()
        .unwrap()
        .extend((658..1025).map(|n| {
            let mut next = file.clone();
            next["path"] = format!("file-{n}.dll").into();
            next
        }));
    assert!(parse(&serde_json::to_vec(&value).unwrap(), "codex").is_err());
    let mut wrong = manifest();
    wrong["providers"][0]["files"][0]["root"] = "powerShellRuntime".into();
    assert!(parse_value(&wrong).is_ok());
    wrong["providers"][0]["provider"] = "claurst".into();
    wrong["providers"][0]["credentialContract"] = "claurst-daemon-model-v1".into();
    assert!(parse(&serde_json::to_vec(&wrong).unwrap(), "claurst").is_err());
    let mut data = manifest();
    data["providers"][0]["provider"] = "codex".into();
    data["providers"][0]["credentialContract"] = "codex-file-responses-v1".into();
    data["providers"][0]["files"][0] = json!({"root":"osGlobalizationData",
        "path":"Globalization/Sorting/sortdefault.nls","sha256":"a".repeat(64)});
    assert!(parse(&serde_json::to_vec(&data).unwrap(), "codex").is_ok());
    data["providers"][0]["files"][0]["path"] = "System32/private.dll".into();
    assert!(parse(&serde_json::to_vec(&data).unwrap(), "codex").is_err());
    data["providers"][0]["files"][0] = json!({"root":"powerShellLauncher",
        "path":"pwsh.exe","sha256":"a".repeat(64)});
    assert!(parse(&serde_json::to_vec(&data).unwrap(), "codex").is_ok());
    data["providers"][0]["files"][0]["path"] = "other.exe".into();
    assert!(parse(&serde_json::to_vec(&data).unwrap(), "codex").is_err());
}

#[test]
fn command_shim_contract_requires_all_three_names_from_one_reviewed_image() {
    let mut value = manifest();
    for shim in COMMAND_SHIMS {
        value["providers"][0]["files"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "root":"powerShellLauncher","path":shim,"sha256":"e".repeat(64)
            }));
    }
    let complete = parse_value(&value).unwrap();
    assert!(require_command_shims(&complete).is_ok());
    let without = |name: &str| {
        let mut contract = complete.clone();
        contract.files.retain(|file| file.path != name);
        contract
    };
    for shim in COMMAND_SHIMS {
        assert!(require_command_shims(&without(shim)).is_err(), "{shim}");
    }
    let mut divergent = complete.clone();
    divergent.files.last_mut().unwrap().sha256 = "f".repeat(64);
    assert!(require_command_shims(&divergent).is_err());
    assert!(require_command_shims(&parse_value(&manifest()).unwrap()).is_err());
}

#[test]
fn unregistered_provider_identity_has_no_manifest_contract() {
    let mut value = manifest();
    value["providers"][0]["provider"] = "gemini".into();
    assert!(parse(&serde_json::to_vec(&value).unwrap(), "gemini").is_err());
}

#[test]
fn traversal_aliases_and_unbounded_or_duplicated_paths_fail_closed() {
    for path in [
        "../private/auth.json",
        "C:/private/auth.json",
        "bin\\node.exe",
        "bin/node.exe:stream",
        "/node.exe",
        "bin/./node.exe",
        "bin/node.exe.",
    ] {
        let mut value = manifest();
        value["providers"][0]["files"][0]["path"] = path.into();
        assert!(parse_value(&value).is_err(), "{path}");
    }
    let mut value = manifest();
    let repeated = value["providers"][0]["files"][0].clone();
    value["providers"][0]["files"]
        .as_array_mut()
        .unwrap()
        .push(repeated);
    assert!(parse_value(&value).is_err());
    let mut value = manifest();
    value["providers"][0]["osLibraries"] = json!(["bcrypt.dll", "BCRYPT.DLL"]);
    assert!(parse_value(&value).is_err());
}

#[test]
fn missing_identity_and_unknown_credential_contracts_are_not_factory_support() {
    for key in [
        "nodeSha256",
        "packageName",
        "packageVersion",
        "packageIntegrity",
    ] {
        let mut value = manifest();
        value["providers"][0].as_object_mut().unwrap().remove(key);
        assert!(parse_value(&value).is_err(), "{key}");
    }
    let mut value = manifest();
    value["providers"][0]["credentialContract"] = "unreviewed-provider-transport".into();
    assert!(parse_value(&value).is_err());
}

#[test]
fn claurst_uses_its_own_native_image_without_inheriting_node_authority() {
    let value = json!({"version":1,"providers":[{
        "provider":"claurst","platform":"win32-x64", "executableSha256":"a".repeat(64),
        "credentialContract":"claurst-daemon-model-v1", "osLibraries":["bcrypt.dll"],
        "files":[{"root":"gitRuntime","path":"usr/bin/bash.exe","sha256":"b".repeat(64)}]
    }]});
    assert!(parse(&serde_json::to_vec(&value).unwrap(), "claurst").is_ok());
    let mut wrong = value;
    wrong["providers"][0]["files"][0]["root"] = "nodeRuntime".into();
    assert!(parse(&serde_json::to_vec(&wrong).unwrap(), "claurst").is_err());
}

#[test]
fn locked_file_identity_rejects_tampering_and_prevents_live_replacement() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("runtime.bin");
    std::fs::write(&path, b"reviewed fixture").unwrap();
    let path = path.canonicalize().unwrap();
    let expected = hex::encode(Sha256::digest(b"reviewed fixture"));
    let (mut grants, mut handles) = (Vec::new(), Vec::new());
    super::super::pin(&path, &expected, &mut grants, &mut handles).unwrap();
    assert!(std::fs::write(&path, b"substitute").is_err());
    assert!(std::fs::remove_file(&path).is_err());
    drop(handles);
    std::fs::write(&path, b"substitute").unwrap();
    assert!(super::super::pin(&path, &expected, &mut Vec::new(), &mut Vec::new()).is_err());
}

#[test]
fn resource_path_accepts_windows_case_but_rejects_real_redirection() {
    let owned = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    std::fs::write(owned.path().join("MixedCase.bin"), b"owned").unwrap();
    let root = owned.path().canonicalize().unwrap();
    let actual = resource_path(&root, "mixedcase.bin").unwrap();
    assert_eq!(
        actual,
        owned.path().join("MixedCase.bin").canonicalize().unwrap()
    );
    assert!(resource_path(&root, "absent.bin").is_err());
    std::fs::write(target.path().join("payload.bin"), b"outside").unwrap();
    let junction = owned.path().join("junction");
    std::os::windows::fs::symlink_dir(target.path(), &junction)
        .expect("owned reparse setup requires existing DeveloperMode");
    assert!(resource_path(&root, "junction/payload.bin").is_err());
    assert_eq!(
        std::fs::read(target.path().join("payload.bin")).unwrap(),
        b"outside"
    );
}
