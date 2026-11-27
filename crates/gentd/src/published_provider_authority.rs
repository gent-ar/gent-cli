use std::io::Read;

use crate::{
    ordinary_authority_release::{MAX_BYTES, OrdinaryAuthorityReleaseError},
    standalone_authority_release::StandaloneAuthorityRelease,
};

pub(crate) const PUBLISHED_AUTHORITY_URL: &str = "https://github.com/gent-ar/gent-cli/releases/download/provider-authority/ordinary-authority.json";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PublishedAuthorityOutcome {
    Staged,
    Current,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PublishedAuthorityError {
    #[error("published provider authority could not be fetched: {0}")]
    Fetch(String),
    #[error("published provider authority exceeds the bounded size")]
    TooLarge,
    #[error("published provider authority was refused: {0}")]
    Refused(#[from] OrdinaryAuthorityReleaseError),
    #[error("published provider authority could not be stored: {0}")]
    Store(std::io::Error),
}

#[derive(Clone, Debug)]
pub(crate) struct PublishedProviderAuthority {
    release: StandaloneAuthorityRelease,
    url: String,
}

impl PublishedProviderAuthority {
    pub(crate) fn new(release: StandaloneAuthorityRelease, url: impl Into<String>) -> Self {
        Self {
            release,
            url: url.into(),
        }
    }

    pub(crate) fn update(&self) -> Result<PublishedAuthorityOutcome, PublishedAuthorityError> {
        let document = fetch(&self.url)?;
        self.release
            .stage_published(&document, crate::startup::unix_seconds())
    }

    pub(crate) fn update_in_background(&self) {
        let published = self.clone();
        std::thread::spawn(move || {
            if let Err(error) = published.update() {
                eprintln!("gentd kept its current provider authority: {error}");
            }
        });
    }
}

fn fetch(url: &str) -> Result<Vec<u8>, PublishedAuthorityError> {
    let failed = |error: reqwest::Error| PublishedAuthorityError::Fetch(error.to_string());
    let response = reqwest::blocking::get(url)
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(failed)?;
    let mut document = Vec::new();
    response
        .take(MAX_BYTES + 1)
        .read_to_end(&mut document)
        .map_err(|error| PublishedAuthorityError::Fetch(error.to_string()))?;
    (document.len() as u64 <= MAX_BYTES)
        .then_some(document)
        .ok_or(PublishedAuthorityError::TooLarge)
}

#[cfg(test)]
#[path = "published_provider_authority_tests.rs"]
mod tests;
