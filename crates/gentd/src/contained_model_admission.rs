use super::ModelError;
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
pub(super) fn verify_helper_manifest(manifest: &str, helper: &str) -> Result<(), ModelError> {
    if !digest(manifest) || !digest(helper) {
        return Err(ModelError::Request);
    }
    if manifest != helper {
        return Err(ModelError::HelperManifestMismatch);
    }
    Ok(())
}
pub(crate) fn packaged_model_error(error: ModelError) -> &'static str {
    match error {
        ModelError::HelperManifestMismatch => {
            "Claurst model runtime/helper identity mismatch; repair the runtime bundle"
        }
        _ => {
            "Claurst model runtime admission failed; verify manifest, executable and OS dependency pins"
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn valid_helper_identity_must_match_the_sealed_model_manifest() {
        assert!(verify_helper_manifest(&"a".repeat(64), &"a".repeat(64)).is_ok());
        assert_eq!(
            verify_helper_manifest(&"a".repeat(64), &"b".repeat(64)),
            Err(ModelError::HelperManifestMismatch)
        );
    }
    #[test]
    fn invalid_equal_inputs_never_bypass_digest_validation() {
        assert_eq!(verify_helper_manifest("", ""), Err(ModelError::Request));
        assert_eq!(
            verify_helper_manifest(&"A".repeat(64), &"A".repeat(64)),
            Err(ModelError::Request)
        );
    }
    #[test]
    fn mismatch_has_actionable_static_diagnostics_without_input_paths() {
        let message = packaged_model_error(ModelError::HelperManifestMismatch);
        assert!(message.contains("identity mismatch") && message.contains("repair"));
        assert!(message.len() < 160 && !message.contains(":\\"));
    }
}
