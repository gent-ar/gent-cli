use std::path::PathBuf;

use clap::Parser;
use gent_testkit::validate_ipc_fixture_manifest;

#[derive(Debug, Parser)]
#[command(about = "Validates language-neutral local IPC contract fixtures")]
struct Args {
    #[arg(default_value = "fixtures/ipc-contract/manifest.json")]
    manifest: PathBuf,
}

fn main() -> Result<(), String> {
    let args = Args::parse();
    validate_ipc_fixture_manifest(&args.manifest)?;
    println!(
        "IPC contract fixtures are valid: {}",
        args.manifest.display()
    );
    Ok(())
}
