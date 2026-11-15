#[derive(Clone, Copy, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(
    tag = "phase",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProviderInstallProgress {
    Preparing,
    Downloading {
        received_bytes: u64,
        total_bytes: Option<u64>,
    },
    DownloadFailed,
    VerifyingArchive,
    Installing,
    VerifyingInstall,
}
