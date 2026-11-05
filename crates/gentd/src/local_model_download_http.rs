use async_trait::async_trait;

use super::{DownloadRequest, ModelDownloadError, ModelDownloadResponse, ModelDownloadTransport};

#[derive(Clone, Debug)]
pub(crate) struct ReqwestModelDownloadTransport {
    client: reqwest::Client,
}

impl ReqwestModelDownloadTransport {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl ModelDownloadTransport for ReqwestModelDownloadTransport {
    async fn get(
        &self,
        request: DownloadRequest,
    ) -> Result<Box<dyn ModelDownloadResponse>, ModelDownloadError> {
        let mut request_builder = self.client.get(request.url);
        if let Some(start) = request.range_start {
            request_builder =
                request_builder.header(reqwest::header::RANGE, format!("bytes={start}-"));
        }
        let response = request_builder
            .send()
            .await
            .map_err(|error| ModelDownloadError::Transport(error.to_string()))?;
        Ok(Box::new(ReqwestModelDownloadResponse { response }))
    }
}

#[derive(Debug)]
struct ReqwestModelDownloadResponse {
    response: reqwest::Response,
}

#[async_trait]
impl ModelDownloadResponse for ReqwestModelDownloadResponse {
    fn status(&self) -> u16 {
        self.response.status().as_u16()
    }

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ModelDownloadError> {
        self.response
            .chunk()
            .await
            .map(|chunk| chunk.map(|chunk| chunk.to_vec()))
            .map_err(|error| ModelDownloadError::Transport(error.to_string()))
    }
}
