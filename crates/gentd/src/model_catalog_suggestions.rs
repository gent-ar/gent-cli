use std::{
    collections::HashSet,
    io::Read,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use gent_protocol::model_catalog::CatalogModel;
use serde::Deserialize;

use super::service::{provider_default_model, suggested_model};

const CATALOG_URL: &str =
    "https://raw.githubusercontent.com/gent-ar/gent-cli/main/crates/gentd/provider-models.json";
const SHIPPED_CATALOG: &str = include_str!("../provider-models.json");
static ONLINE_CATALOG: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static ONLINE_REFRESH: OnceLock<()> = OnceLock::new();

#[derive(Deserialize)]
struct Catalog {
    version: u32,
    providers: Providers,
}

#[derive(Deserialize)]
struct Providers {
    claude: Vec<Model>,
    codex: Vec<Model>,
}

#[derive(Deserialize)]
struct Model {
    id: String,
    label: String,
}

pub(crate) fn models(provider: &str) -> Vec<CatalogModel> {
    refresh_online_catalog();
    let catalog = cached_online_catalog()
        .and_then(|body| parse_catalog(&body))
        .or_else(|| parse_catalog(SHIPPED_CATALOG));
    let entries = catalog.map(|catalog| match provider {
        "claude" => catalog.providers.claude,
        "codex" => catalog.providers.codex,
        _ => Vec::new(),
    });
    let mut models = vec![provider_default_model()];
    if let Some(entries) = entries {
        models.extend(
            entries
                .into_iter()
                .map(|entry| suggested_model(&entry.id, &entry.label)),
        );
    }
    models
}

pub(crate) fn merge(provider: &str, mut models: Vec<CatalogModel>) -> Vec<CatalogModel> {
    for suggested in self::models(provider) {
        if !models.iter().any(|model| model.id == suggested.id) {
            models.push(suggested);
        }
    }
    models
}

fn refresh_online_catalog() {
    if ONLINE_REFRESH.set(()).is_err() {
        return;
    }
    std::thread::spawn(|| {
        let Some(body) = online_catalog() else {
            return;
        };
        if parse_catalog(&body).is_some() {
            if let Ok(mut catalog) = ONLINE_CATALOG.get_or_init(|| Mutex::new(None)).lock() {
                *catalog = Some(body);
            }
        }
    });
}

fn cached_online_catalog() -> Option<String> {
    ONLINE_CATALOG
        .get()
        .and_then(|catalog| catalog.lock().ok()?.clone())
}

fn online_catalog() -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    let response = client
        .get(CATALOG_URL)
        .send()
        .ok()?
        .error_for_status()
        .ok()?;
    let mut body = String::new();
    response.take(65_537).read_to_string(&mut body).ok()?;
    (body.len() <= 65_536).then_some(body)
}

fn parse_catalog(body: &str) -> Option<Catalog> {
    let catalog: Catalog = serde_json::from_str(body).ok()?;
    if catalog.version != 1
        || !valid_models(&catalog.providers.claude)
        || !valid_models(&catalog.providers.codex)
    {
        return None;
    }
    Some(catalog)
}

fn valid_models(models: &[Model]) -> bool {
    let mut ids = HashSet::new();
    !models.is_empty()
        && models.len() <= 100
        && models.iter().all(|model| {
            !model.id.is_empty()
                && model.id.len() <= 64
                && model
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-._[]".contains(&byte))
                && model.label.len() <= 64
                && !model.label.trim().is_empty()
                && ids.insert(model.id.as_str())
        })
}

#[cfg(test)]
mod tests {
    use super::{SHIPPED_CATALOG, parse_catalog};

    #[test]
    fn shipped_catalog_is_valid_and_contains_both_providers() {
        let catalog = parse_catalog(SHIPPED_CATALOG).unwrap();
        assert!(catalog.providers.claude.len() > 3);
        assert!(catalog.providers.codex.len() > 4);
    }

    #[test]
    fn invalid_remote_catalog_is_rejected() {
        assert!(
            parse_catalog(
                r#"{"version":1,"providers":{"claude":[{"id":"bad/id","label":"Bad"}],"codex":[]}}"#
            )
            .is_none()
        );
    }
}
