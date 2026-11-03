"""Publish every signed dependency required by standalone installers."""
import argparse
from pathlib import Path
import shutil
import subprocess

ASSETS = {
    "install.sh": "gent-install.sh",
    "install.ps1": "gent-install.ps1",
    "activate-install.py": "gent-activate-install.py",
    "activate_install_support.py": "gent-activate-install-support.py",
    "install_archive_validation.ps1": "gent-install-support.ps1",
    "gent-auto-update.py": "gent-auto-update.py",
    "gent-auto-update.ps1": "gent-auto-update.ps1",
    "supervise-runtime-activation.py": "gent-supervise-runtime-activation.py",
}

def stage(source: Path, destination: Path, signer=subprocess.run) -> None:
    for name in ASSETS:
        path = source / name
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"installer dependency is missing or unsafe: {name}")
    destination.mkdir(parents=True, exist_ok=True)
    for name, asset in ASSETS.items():
        output = destination / asset
        shutil.copyfile(source / name, output)
        signer(["cosign", "sign-blob", "--yes", "--bundle", str(output) + ".sigstore.json", str(output)], check=True)
        if not Path(str(output) + ".sigstore.json").is_file():
            raise ValueError(f"installer dependency signing did not produce a bundle: {asset}")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    arguments = parser.parse_args()
    stage(Path(__file__).resolve().parent, arguments.destination)
