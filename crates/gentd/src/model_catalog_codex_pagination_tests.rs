use std::sync::Mutex;

use gent_drivers::{
    interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal},
    supervisor::ProviderProcess,
};
use serde_json::{Value, json};

use super::{FIRST_PAGE_REQUEST_ID, MAX_PAGES, ModelListPages, ProbeStep};

#[test]
fn provider_order_and_default_are_preserved_without_model_name_ranking() {
    let entries = [
        json!({"model": "provider-future", "displayName": "Provider recommended", "isDefault": true}),
        json!({"model": "provider-mini", "displayName": "Smaller provider option", "isDefault": false}),
    ];
    let models = entries
        .iter()
        .filter_map(super::codex_model)
        .collect::<Vec<_>>();
    assert_eq!(
        models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["provider-future", "provider-mini"]
    );
    assert!(models[0].is_default);
    assert!(!models[1].is_default);
    assert_eq!(models[0].label, "Provider recommended");
}

#[derive(Default)]
struct ModelEndpoint(Mutex<Vec<Value>>);

impl ProcessTreeControl for ModelEndpoint {
    fn signal_tree(&self, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        unreachable!()
    }
}

impl ProviderProcess for ModelEndpoint {
    fn write_frame(&self, frame: &[u8]) -> Result<(), ProcessTreeError> {
        self.0
            .lock()
            .unwrap()
            .push(serde_json::from_slice(frame).unwrap());
        Ok(())
    }

    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        unreachable!()
    }
}

#[test]
fn a_bounded_but_incomplete_catalog_is_rejected_instead_of_silently_truncated() {
    let endpoint = ModelEndpoint::default();
    let mut listing = ModelListPages::default();
    for page in 0..MAX_PAGES {
        let result = listing.accept(
            &json!({"id": FIRST_PAGE_REQUEST_ID + page, "result": {
                "data": [{"model": format!("provider-model-{page}")}],
                "nextCursor": format!("cursor-{}", page + 1)
            }}),
            &endpoint,
        );
        if page + 1 == MAX_PAGES {
            assert!(result.err().unwrap().contains("catalog is incomplete"));
        } else {
            assert!(matches!(result.unwrap(), ProbeStep::Continue));
        }
    }
    assert_eq!(
        endpoint.0.lock().unwrap().len(),
        usize::try_from(MAX_PAGES - 1).unwrap()
    );
}

#[test]
fn the_last_allowed_complete_page_keeps_every_provider_model_in_order() {
    let endpoint = ModelEndpoint::default();
    let mut listing = ModelListPages::default();
    for page in 0..MAX_PAGES {
        let cursor = (page + 1 < MAX_PAGES).then(|| format!("cursor-{}", page + 1));
        let result = listing
            .accept(
                &json!({"id": FIRST_PAGE_REQUEST_ID + page, "result": {
                    "data": [{"model": format!("provider-model-{page}")}], "nextCursor": cursor
                }}),
                &endpoint,
            )
            .unwrap();
        if page + 1 == MAX_PAGES {
            let ProbeStep::Done(models) = result else {
                panic!("complete final page was not returned")
            };
            assert_eq!(
                models
                    .iter()
                    .map(|model| model.id.clone())
                    .collect::<Vec<_>>(),
                (0..MAX_PAGES)
                    .map(|page| format!("provider-model-{page}"))
                    .collect::<Vec<_>>()
            );
        } else {
            assert!(matches!(result, ProbeStep::Continue));
        }
    }
}

#[test]
fn captured_managed_and_new_binary_catalogs_keep_every_public_model() {
    for capture in [
        include_str!("../fixtures/model_catalog/public-codex-0.153.4.json"),
        include_str!("../fixtures/model_catalog/public-codex-0.165.0.json"),
    ] {
        let fixture: Value = serde_json::from_str(capture).unwrap();
        let endpoint = ModelEndpoint::default();
        let mut listing = ModelListPages::default();
        let mut actual = None;
        for (page, result) in fixture["pages"].as_array().unwrap().iter().enumerate() {
            if let ProbeStep::Done(models) = listing
                .accept(
                    &json!({"id": FIRST_PAGE_REQUEST_ID + page as u64, "result": result}),
                    &endpoint,
                )
                .unwrap()
            {
                actual = Some(models);
            }
        }
        let ids = actual
            .unwrap()
            .into_iter()
            .map(|model| Value::String(model.id))
            .collect::<Vec<_>>();
        assert_eq!(ids, *fixture["expectedIds"].as_array().unwrap());
    }
}
