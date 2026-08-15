use std::path::PathBuf;

use clap::Parser;
use gent_testkit::validate_public_driver_manifest;

#[derive(Debug, Parser)]
#[command(
    about = "Validates public-driver transcript capture readiness without fabricating evidence"
)]
struct Args {
    #[arg(default_value = "fixtures/public-driver-transcripts/manifest.yml")]
    manifest: PathBuf,
    /// Require each cell to be a live recording or an explicit recorded absence.
    #[arg(long)]
    require_live: bool,
}

fn main() -> Result<(), String> {
    let args = Args::parse();
    validate_public_driver_manifest(&args.manifest, args.require_live)?;
    println!(
        "public-driver transcript manifest is valid: {}",
        args.manifest.display()
    );
    Ok(())
}
