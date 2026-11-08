use std::{
    path::Path,
    process::{Child, Command, Stdio},
};

pub struct Gentd(pub Child);

impl Drop for Gentd {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn gentd(data_dir: &Path) -> Command {
    std::fs::create_dir_all(data_dir).unwrap();
    std::fs::write(
        data_dir.join("model-catalog.json"),
        serde_json::json!({
            "defaultSelection": null,
            "declinedDownloads": [gent_protocol::DEFAULT_LOCAL_MODEL_ID],
        })
        .to_string(),
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_gentd"));
    command
        .arg("--data-dir")
        .arg(data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}
