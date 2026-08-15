use std::path::PathBuf;

use clap::Parser;
use gent_testkit::validate_evidence_manifest;

#[derive(Debug, Parser)]
#[command(
    about = "Validates the Gent coverage-manifest evidence graph without fabricating evidence"
)]
struct Args {
    #[arg(default_value = "fixtures/coverage-manifest.yml")]
    manifest: PathBuf,
    /// Enforce the deliberately stricter, post-phase-0 authority-transfer evidence requirements.
    #[arg(long)]
    authority_transfer: bool,
}

fn main() -> Result<(), String> {
    let args = Args::parse();
    validate_evidence_manifest(&args.manifest, args.authority_transfer)?;
    println!(
        "coverage manifest is structurally valid: {}",
        args.manifest.display()
    );
    Ok(())
}
