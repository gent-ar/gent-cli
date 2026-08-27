use clap::Parser;

use super::{Args, CommandLine};
use crate::local_models_cli::LocalModelsCommand;

#[test]
fn local_models_parse_catalogue_status_and_consented_download_commands() {
    let list = Args::try_parse_from(["gent", "models", "list"]).unwrap();
    assert!(matches!(
        list.command,
        Some(CommandLine::Models {
            action: LocalModelsCommand::List
        })
    ));
    let model = "qwen2-5-coder-7b-instruct-q4-k-m";
    let status = Args::try_parse_from(["gent", "models", "status", model]).unwrap();
    assert!(matches!(
        status.command,
        Some(CommandLine::Models {
            action: LocalModelsCommand::Status { model_id }
        }) if model_id == model
    ));
    let download = Args::try_parse_from(["gent", "models", "download", model]).unwrap();
    assert!(matches!(
        download.command,
        Some(CommandLine::Models {
            action: LocalModelsCommand::Download { model_id }
        }) if model_id == model
    ));
}
