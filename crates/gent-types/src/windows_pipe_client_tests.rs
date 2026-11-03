#[test]
fn candidates_match_shared_normative_vectors_without_changing_raw_hash() {
    let contract: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/ipc/windows-client-endpoint-candidates-v1.json"
    ))
    .unwrap();
    for vector in contract["vectors"].as_array().unwrap() {
        let input = vector["input"].as_str().unwrap();
        let path = std::path::Path::new(input);
        assert_eq!(
            crate::windows_pipe_name(path),
            vector["legacyPipe"].as_str().unwrap(),
            "{input:?}"
        );
        let expected = vector["clientPipes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            super::windows_pipe_client_candidates(path),
            expected,
            "{input:?}"
        );
        assert!(expected.len() <= 3);
        assert_eq!(expected[0], crate::windows_pipe_name(path));
        assert_eq!(
            expected
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            expected.len()
        );
    }
}
#[cfg(windows)]
#[test]
fn ill_formed_utf16_never_generates_lossy_aliases() {
    use std::os::windows::ffi::OsStringExt;
    let contract: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/ipc/windows-client-endpoint-candidates-v1.json"
    ))
    .unwrap();
    for vector in contract["illFormedUtf16WindowsOnly"].as_array().unwrap() {
        let units = vector["units"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| u16::try_from(value.as_u64().unwrap()).unwrap())
            .collect::<Vec<_>>();
        let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&units));
        assert!(path.to_str().is_none());
        let expected = vector["clientPipes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(super::windows_pipe_client_candidates(&path), expected);
        assert_eq!(expected.len(), 1);
    }
}
