use super::*;

#[tokio::test]
async fn local_models_report_size_and_install_state_with_the_small_model_as_default() {
    let root = tempfile::tempdir().unwrap();
    let models = crate::local_model_jobs::tests::models(root.path())
        .with_download_transport(Arc::new(crate::local_model_jobs::tests::BlockingTransport));
    let source = LocalModelSource {
        models: models.clone(),
    };
    let listing = source.load().unwrap();
    assert_eq!(listing.availability, ProviderAvailability::Ready);
    let small = &listing.models[0];
    assert_eq!(small.id, "qwen3-1-7b-q4-k-m");
    assert!(small.is_default);
    assert_eq!(
        small.local,
        Some(LocalModelAvailability {
            size_bytes: 1_282_439_264,
            install: LocalModelInstallState::NotInstalled,
            removable: false,
            file_name: None,
        })
    );
    assert!(listing.models[1..].iter().all(|model| !model.is_default));

    let plan = models.provisioner.plan("qwen3-8b-q4-k-m").unwrap();
    models.provisioner.ensure_storage(&plan).unwrap();
    fs::write(&plan.partial_destination, [1_u8; 16]).unwrap();
    let inactive = source.load().unwrap();
    assert_eq!(
        inactive.models[1].local.as_ref().unwrap().install,
        LocalModelInstallState::NotInstalled
    );
    models
        .downloads
        .start(
            "qwen3-8b-q4-k-m",
            crate::local_model_jobs::DownloadHolder::Background,
        )
        .unwrap();
    let downloading = source.load().unwrap();
    assert_eq!(
        downloading.models[1].local.as_ref().unwrap().install,
        LocalModelInstallState::Downloading {
            downloaded_bytes: 16,
            total_bytes: plan.expected_bytes,
        }
    );

    let ready = models.provisioner.plan("qwen3-1-7b-q4-k-m").unwrap();
    models.provisioner.ensure_storage(&ready).unwrap();
    fs::File::create(&ready.destination)
        .unwrap()
        .set_len(ready.expected_bytes)
        .unwrap();
    crate::local_model_integrity::remember_sha256(&ready.destination, &ready.expected_sha256);
    assert_eq!(
        source.load().unwrap().models[0]
            .local
            .as_ref()
            .unwrap()
            .install,
        LocalModelInstallState::Ready {
            size_bytes: ready.expected_bytes,
        }
    );
}

#[test]
fn a_provider_that_does_not_answer_within_its_budget_times_out_as_retryable() {
    let root = tempfile::tempdir().unwrap();
    let source = CodexModelSource {
        lifecycle_control: None,
        executables: crate::provider_executables::ProviderExecutables::explicit(
            None,
            Some(executable(
                root.path(),
                "codex",
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then exit 0; fi\nexec sleep 30\n",
            )),
        ),
        auth: Arc::new(FixedAuth(ProviderAuthLifecycle::Authenticated)),
        timeout: Duration::from_millis(300),
    };
    assert!(matches!(
        source.load(),
        Err(ProviderLaunchError::TimedOut(message)) if message.contains("in time")
    ));
}

struct SettlingAuth(std::sync::atomic::AtomicUsize);

impl ProviderAuthPort for SettlingAuth {
    fn exchange(&self, _: ProviderAuthFrame) -> Result<ProviderAuthFrame, String> {
        Err("unexpected auth frame".into())
    }

    fn check_status(&self, provider: gent_types::ProviderAuthProvider) -> ProviderAuthStatus {
        let reads = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        FixedAuth(if reads < 2 {
            ProviderAuthLifecycle::Checking
        } else {
            ProviderAuthLifecycle::Authenticated
        })
        .check_status(provider)
    }
}

#[test]
fn a_provider_account_still_being_checked_is_awaited_instead_of_reported_signed_out() {
    let source = CodexModelSource {
        lifecycle_control: None,
        executables: crate::provider_executables::ProviderExecutables::explicit(None, None),
        auth: Arc::new(SettlingAuth(std::sync::atomic::AtomicUsize::new(0))),
        timeout: Duration::from_secs(1),
    };
    assert_eq!(
        source.load().unwrap().availability,
        ProviderAvailability::Ready
    );
}
