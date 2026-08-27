use crate::local_model_catalog::LocalModelCatalog;
use crate::local_model_download::{
    DownloadRequest, ModelDownloadError, ModelDownloadProgress, ModelDownloadResponse,
    ModelDownloadTransport, download_model,
};
use crate::local_model_provisioning::{LocalModelDownloadPlan, LocalModelProvisioner};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

#[derive(Clone, Debug)]
struct FakeTransport {
    request: Arc<Mutex<Option<DownloadRequest>>>,
    status: u16,
    chunks: Vec<Vec<u8>>,
}

#[async_trait]
impl ModelDownloadTransport for FakeTransport {
    async fn get(
        &self,
        request: DownloadRequest,
    ) -> Result<Box<dyn ModelDownloadResponse>, ModelDownloadError> {
        *self.request.lock().unwrap() = Some(request);
        Ok(Box::new(FakeResponse {
            status: self.status,
            chunks: self.chunks.clone(),
        }))
    }
}

#[derive(Debug)]
struct FakeResponse {
    status: u16,
    chunks: Vec<Vec<u8>>,
}

#[async_trait]
impl ModelDownloadResponse for FakeResponse {
    fn status(&self) -> u16 {
        self.status
    }

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ModelDownloadError> {
        Ok((!self.chunks.is_empty()).then(|| self.chunks.remove(0)))
    }
}

fn plan(expected_bytes: u64) -> (tempfile::TempDir, LocalModelDownloadPlan) {
    let directory = tempdir().unwrap();
    let provisioner =
        LocalModelProvisioner::new(directory.path(), LocalModelCatalog::shipped().unwrap());
    let mut plan = provisioner.plan("qwen3-1-7b-q4-k-m").unwrap();
    plan.expected_bytes = expected_bytes;
    plan.expected_sha256 = hex(&Sha256::digest(b"abcde"));
    (directory, plan)
}

fn hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

#[tokio::test]
async fn resumes_partial_download_reports_progress_and_renames_only_when_complete() {
    let (_directory, plan) = plan(5);
    std::fs::create_dir_all(plan.partial_destination.parent().unwrap()).unwrap();
    std::fs::write(&plan.partial_destination, b"ab").unwrap();
    let captured = Arc::new(Mutex::new(None));
    let transport = FakeTransport {
        request: Arc::clone(&captured),
        status: 206,
        chunks: vec![b"c".to_vec(), b"de".to_vec()],
    };
    let mut progress = Vec::new();
    let path = download_model(&plan, &transport, |event| progress.push(event))
        .await
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"abcde");
    assert!(!plan.partial_destination.exists());
    assert_eq!(
        captured.lock().unwrap().as_ref().unwrap().range_start,
        Some(2)
    );
    assert!(matches!(
        progress.last(),
        Some(ModelDownloadProgress::Complete { .. })
    ));
}

#[tokio::test]
async fn coalesces_large_download_progress_without_losing_monotonic_completion() {
    let expected_bytes = 3 * 1_048_576_u64;
    let (_directory, mut plan) = plan(expected_bytes);
    let bytes = vec![b'x'; usize::try_from(expected_bytes).unwrap()];
    plan.expected_sha256 = hex(&Sha256::digest(&bytes));
    let transport = FakeTransport {
        request: Arc::new(Mutex::new(None)),
        status: 200,
        chunks: bytes.chunks(16_384).map(ToOwned::to_owned).collect(),
    };
    let mut progress = Vec::new();
    download_model(&plan, &transport, |event| progress.push(event))
        .await
        .unwrap();
    let advanced = progress
        .iter()
        .filter_map(|event| match event {
            ModelDownloadProgress::Advanced {
                downloaded_bytes, ..
            } => Some(*downloaded_bytes),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(advanced.len() <= 3);
    assert_eq!(advanced.last(), Some(&expected_bytes));
    assert!(advanced.windows(2).all(|window| window[0] < window[1]));
    assert!(matches!(
        progress.last(),
        Some(ModelDownloadProgress::Complete { .. })
    ));
}

#[tokio::test]
async fn range_refusal_preserves_the_partial_for_a_future_resume() {
    let (_directory, plan) = plan(5);
    std::fs::create_dir_all(plan.partial_destination.parent().unwrap()).unwrap();
    std::fs::write(&plan.partial_destination, b"ab").unwrap();
    let transport = FakeTransport {
        request: Arc::new(Mutex::new(None)),
        status: 200,
        chunks: vec![b"cde".to_vec()],
    };
    assert_eq!(
        download_model(&plan, &transport, |_| {}).await,
        Err(ModelDownloadError::ResumeRejected(200))
    );
    assert_eq!(std::fs::read(&plan.partial_destination).unwrap(), b"ab");
    assert!(!plan.destination.exists());
}

#[tokio::test]
async fn incomplete_and_oversized_responses_never_publish_a_final_model() {
    let (_directory, plan) = plan(5);
    let incomplete = FakeTransport {
        request: Arc::new(Mutex::new(None)),
        status: 200,
        chunks: vec![b"abc".to_vec()],
    };
    assert!(matches!(
        download_model(&plan, &incomplete, |_| {}).await,
        Err(ModelDownloadError::SizeMismatch { .. })
    ));
    assert!(!plan.destination.exists());
    std::fs::remove_file(&plan.partial_destination).unwrap();
    let oversized = FakeTransport {
        request: Arc::new(Mutex::new(None)),
        status: 200,
        chunks: vec![b"abcdef".to_vec()],
    };
    assert!(matches!(
        download_model(&plan, &oversized, |_| {}).await,
        Err(ModelDownloadError::ExceededApprovedSize { .. })
    ));
    assert!(!plan.destination.exists());
}

#[tokio::test]
async fn matching_size_with_wrong_digest_never_publishes_a_final_model() {
    let (_directory, mut plan) = plan(5);
    plan.expected_sha256 = "a".repeat(64);
    let transport = FakeTransport {
        request: Arc::new(Mutex::new(None)),
        status: 200,
        chunks: vec![b"abcde".to_vec()],
    };
    assert_eq!(
        download_model(&plan, &transport, |_| {}).await,
        Err(ModelDownloadError::DigestMismatch)
    );
    assert!(!plan.partial_destination.exists());
    assert!(!plan.destination.exists());
}

#[tokio::test]
async fn wrong_digest_removes_partial_so_the_next_attempt_starts_clean() {
    let (_directory, mut plan) = plan(5);
    plan.expected_sha256 = "a".repeat(64);
    let failed = FakeTransport {
        request: Arc::new(Mutex::new(None)),
        status: 200,
        chunks: vec![b"abcde".to_vec()],
    };
    assert_eq!(
        download_model(&plan, &failed, |_| {}).await,
        Err(ModelDownloadError::DigestMismatch)
    );
    assert!(!plan.partial_destination.exists());

    plan.expected_sha256 = hex(&Sha256::digest(b"abcde"));
    let captured = Arc::new(Mutex::new(None));
    let retry = FakeTransport {
        request: Arc::clone(&captured),
        status: 200,
        chunks: vec![b"abcde".to_vec()],
    };
    assert!(download_model(&plan, &retry, |_| {}).await.is_ok());
    assert_eq!(captured.lock().unwrap().as_ref().unwrap().range_start, None);
}
