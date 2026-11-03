use gent_protocol::model_catalog::CatalogModel;

use super::service::{PROVIDER_DEFAULT_MODEL, provider_default_model};

pub(crate) fn discovered(mut models: Vec<CatalogModel>) -> Vec<CatalogModel> {
    if !models
        .iter()
        .any(|model| model.id == PROVIDER_DEFAULT_MODEL)
    {
        let mut default = provider_default_model();
        default.is_default = !models.iter().any(|model| model.is_default);
        models.push(default);
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;
    use gent_types::AgentChatEffort;

    #[test]
    fn no_install_offers_only_provider_default() {
        assert_eq!(discovered(Vec::new()), vec![provider_default_model()]);
    }

    #[test]
    fn discovered_models_keep_provider_metadata_without_curated_additions() {
        let mut model = provider_default_model();
        model.id = "future-provider-model".into();
        model.label = "Provider supplied name".into();
        model.description = Some("Provider supplied details".into());
        model.efforts = vec![AgentChatEffort::High];
        model.default_effort = Some(AgentChatEffort::High);
        let result = discovered(vec![model.clone()]);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], model);
        assert_eq!(result[1].id, PROVIDER_DEFAULT_MODEL);
        assert!(!result[1].is_default);
    }

    #[test]
    fn provider_default_metadata_is_preserved_without_duplicate_alias() {
        let mut model = provider_default_model();
        model.label = "Provider recommended".into();
        model.efforts = vec![AgentChatEffort::Medium];
        assert_eq!(discovered(vec![model.clone()]), vec![model]);
    }
}
