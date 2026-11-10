use super::{CloudSpeechProvider, LocalSpeechEngine, LocalSpeechModel, SpeechCatalogError};

impl CloudSpeechProvider {
    pub(super) fn validate(&self) -> Result<(), SpeechCatalogError> {
        let mut models = Vec::new();
        for model in &self.models {
            if !model_id(&model.id) || !label(&model.label, 160) {
                return Err(SpeechCatalogError::InvalidEntry(model.id.clone()));
            }
            unique(&mut models, &model.id)?;
        }
        let default_listed = match &self.default_model_id {
            Some(default) => models.contains(&default.as_str()),
            None => models.is_empty(),
        };
        if !source_id(&self.id)
            || !label(&self.label, 160)
            || !label(&self.api_key_hint, 64)
            || !https_url(&self.transcription_url)
            || self
                .models_url
                .as_deref()
                .is_some_and(|url| !https_url(url))
            || !default_listed
        {
            return Err(SpeechCatalogError::InvalidEntry(self.id.clone()));
        }
        Ok(())
    }
}

impl LocalSpeechEngine {
    pub(super) fn validate(&self) -> Result<(), SpeechCatalogError> {
        if !source_id(&self.id) || !label(&self.label, 160) || self.models.is_empty() {
            return Err(SpeechCatalogError::InvalidEntry(self.id.clone()));
        }
        let mut ids = Vec::new();
        let mut filenames = Vec::new();
        for model in &self.models {
            model.validate()?;
            unique(&mut ids, &model.id)?;
            unique(&mut filenames, &model.filename)?;
        }
        Ok(())
    }
}

impl LocalSpeechModel {
    pub(super) fn validate(&self) -> Result<(), SpeechCatalogError> {
        let served_file = self
            .url
            .rsplit_once('/')
            .is_some_and(|(_, file)| file == self.filename);
        if !model_id(&self.id)
            || !source_id(&self.size)
            || !label(&self.label, 160)
            || !label(&self.filename, 160)
            || self.filename.contains(['/', '\\'])
            || std::path::Path::new(&self.filename)
                .extension()
                .is_none_or(|extension| extension != "bin")
            || self.size_bytes == 0
            || self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !https_url(&self.url)
            || !served_file
        {
            return Err(SpeechCatalogError::InvalidEntry(self.id.clone()));
        }
        Ok(())
    }
}

pub(super) fn unique<'a>(seen: &mut Vec<&'a str>, id: &'a str) -> Result<(), SpeechCatalogError> {
    if seen.contains(&id) {
        return Err(SpeechCatalogError::Duplicate(id.into()));
    }
    seen.push(id);
    Ok(())
}

fn source_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn model_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._/".contains(&byte))
}

fn https_url(value: &str) -> bool {
    value.len() <= 2_048
        && value
            .strip_prefix("https://")
            .and_then(|rest| rest.split('/').next())
            .is_some_and(|host| !host.is_empty())
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

pub(super) fn label(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}
