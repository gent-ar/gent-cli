use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};
use zeroize::Zeroize;

#[derive(Deserialize, Serialize)]
#[serde(transparent)]
pub struct CredentialSubmission(BTreeMap<String, String>);

impl CredentialSubmission {
    pub fn fields(&self) -> &BTreeMap<String, String> {
        &self.0
    }

    pub fn validate(&self) -> bool {
        !self.0.is_empty()
            && self.0.len() <= 16
            && self.0.iter().all(|(key, value)| {
                super::valid_credential_field_id(key)
                    && !value.is_empty()
                    && value.len() <= 8192
                    && !value.contains('\0')
            })
    }
}

impl Drop for CredentialSubmission {
    fn drop(&mut self) {
        self.0.values_mut().for_each(Zeroize::zeroize);
    }
}

impl fmt::Debug for CredentialSubmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialSubmission([redacted])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_and_private_submission_share_the_same_bounded_field_id_rule() {
        for field in [
            "api-token",
            "service.account:key",
            "workspace/credential",
            "TOKEN_1",
        ] {
            let descriptor = super::super::CredentialField {
                field_id: field.into(),
                label: "API token".into(),
                environment_name: "EXAMPLE_API_TOKEN".into(),
            };
            let submission: CredentialSubmission =
                serde_json::from_value(serde_json::json!({(field):"synthetic-private-value"}))
                    .unwrap();
            assert!(descriptor.validate());
            assert!(submission.validate());
            assert!(submission.fields().contains_key(field));
            assert_eq!(
                format!("{submission:?}"),
                "CredentialSubmission([redacted])"
            );
        }
        for field in [String::new(), "a".repeat(129), "bad field".to_owned()] {
            let descriptor = super::super::CredentialField {
                field_id: field.clone(),
                label: "API token".into(),
                environment_name: "EXAMPLE_API_TOKEN".into(),
            };
            let submission: CredentialSubmission =
                serde_json::from_value(serde_json::json!({(field):"synthetic-private-value"}))
                    .unwrap();
            assert!(!descriptor.validate());
            assert!(!submission.validate());
        }
    }
}
