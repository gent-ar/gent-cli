//! Exact reviewed provider HTTP route forwarding.
use super::{
    EgressError, HeaderName, HeaderValue, Method, ProviderApi, ProviderTarget,
    ReviewedProviderEgress, Url, wire,
};
impl ReviewedProviderEgress {
    pub(super) fn target(&self, method: &str, path: &str) -> Result<Url, EgressError> {
        let (path, query) = path
            .split_once('?')
            .map_or((path, None), |(p, q)| (p, Some(q)));
        let relative = match (self.api, method, path) {
            (ProviderApi::Claude, "POST", "/v1/messages") => "messages",
            (ProviderApi::Claude, "POST", "/v1/messages/count_tokens") => "messages/count_tokens",
            (ProviderApi::Codex, "POST", "/v1/responses") => "responses",
            (ProviderApi::Codex, "POST", "/v1/responses/compact") => "responses/compact",
            (ProviderApi::Claurst, "POST", "/v1/chat/completions") => "chat/completions",
            (_, "GET", "/v1/models") => "models",
            _ => return Err(EgressError::Request),
        };
        if query.is_some() && !(self.api == ProviderApi::Claude && query == Some("beta=true")) {
            return Err(EgressError::Request);
        }
        let ProviderTarget::Http { upstream, .. } = &self.target else {
            return Err(EgressError::Configuration);
        };
        let mut target = upstream.join(relative).map_err(|_| EgressError::Request)?;
        target.set_query(query);
        Ok(target)
    }

    pub(super) fn forward(
        &self,
        request: wire::Request,
    ) -> Result<reqwest::blocking::Response, EgressError> {
        let target = self.target(&request.method, &request.path)?;
        let method =
            Method::from_bytes(request.method.as_bytes()).map_err(|_| EgressError::Request)?;
        let mut headers = self.credentials.clone();
        for (name, value) in request.headers {
            if matches!(name.as_str(), "content-type" | "accept")
                || (name == "anthropic-version" && !headers.contains_key("anthropic-version"))
            {
                headers.insert(
                    HeaderName::from_bytes(name.as_bytes()).map_err(|_| EgressError::Request)?,
                    HeaderValue::from_str(&value).map_err(|_| EgressError::Request)?,
                );
            }
            if name == "anthropic-beta" {
                let mut values = headers
                    .get("anthropic-beta")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .split(',')
                    .chain(value.split(','))
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>();
                values.sort_unstable();
                values.dedup();
                let merged = values.join(",");
                headers.insert(
                    "anthropic-beta",
                    HeaderValue::from_str(&merged).map_err(|_| EgressError::Request)?,
                );
            }
        }
        let ProviderTarget::Http { client, .. } = &self.target else {
            return Err(EgressError::Configuration);
        };
        let response = client
            .request(method, target)
            .headers(headers)
            .body(request.body)
            .send()
            .map_err(|_| EgressError::Transport)?;
        if response.status().is_redirection() || response.status().as_u16() == 101 {
            return Err(EgressError::Transport);
        }
        Ok(response)
    }
}
