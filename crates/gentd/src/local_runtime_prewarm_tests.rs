use std::{fs::File, path::Path};

use super::{files, warm};

fn write(path: &Path, bytes: usize) {
    std::fs::write(path, vec![7_u8; bytes]).unwrap();
}

#[test]
fn reads_the_model_and_only_the_llama_runtime_libraries() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("llama");
    std::fs::create_dir(&runtime).unwrap();
    let model = root.path().join("model.gguf");
    write(&model, 11);
    write(&runtime.join("llama-server.exe"), 1000);
    write(&runtime.join("ggml-cuda.dll"), 3);
    write(&runtime.join("cublasLt64_12.dll"), 5);
    write(&runtime.join("llama.dll"), 7);
    write(&runtime.join("unrelated.dll"), 1000);
    let listed = files(&model, &runtime.join("llama-server.exe"));
    assert_eq!(
        listed
            .iter()
            .map(|path| path.file_name().unwrap().to_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "model.gguf",
            "cublasLt64_12.dll",
            "ggml-cuda.dll",
            "llama.dll"
        ]
    );
    assert_eq!(warm(&listed, &|| false, |path| File::open(path)), 26);
}

#[test]
fn a_missing_model_is_skipped_and_a_stop_request_ends_the_read() {
    let root = tempfile::tempdir().unwrap();
    let library = root.path().join("ggml.dll");
    write(&library, 9);
    let listed = files(
        &root.path().join("absent.gguf"),
        &root.path().join("llama-server.exe"),
    );
    assert_eq!(warm(&listed, &|| false, |path| File::open(path)), 9);
    assert_eq!(warm(&listed, &|| true, |path| File::open(path)), 0);
}

#[test]
fn nothing_is_warmed_until_the_local_model_is_installed() {
    let root = tempfile::tempdir().unwrap();
    let models = crate::local_model_jobs::tests::models(root.path());
    let model_id = gent_protocol::DEFAULT_LOCAL_MODEL_ID;
    assert_eq!(super::ready_model(&models, model_id), None);
    let plan = models.provisioner.plan(model_id).unwrap();
    models.provisioner.ensure_storage(&plan).unwrap();
    File::create(&plan.destination)
        .unwrap()
        .set_len(plan.expected_bytes)
        .unwrap();
    crate::local_model_integrity::remember_sha256(&plan.destination, &plan.expected_sha256);
    assert_eq!(
        super::ready_model(&models, model_id),
        Some(plan.destination.clone())
    );
}
