#!/usr/bin/env python3
import argparse
import json
import os
import pathlib
import shutil
import subprocess
import tempfile

from provider_pins import PINS, require_pinned_authority


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pins", type=pathlib.Path, default=PINS)
    return parser.parse_args()


def main() -> None:
    args = arguments()
    release = pathlib.Path("dist-runtime/authority/ordinary-authority.json").resolve()
    envelope = json.loads(release.read_text(encoding="utf-8"))
    require_pinned_authority(envelope["payload"], args.pins)
    roots = json.loads(pathlib.Path("dist-runtime/authority/root-keys.json").read_text(encoding="utf-8"))
    verification = [
        "--standalone-authority",
        "--verify-standalone-authority-release",
        "--standalone-authority-release",
        str(release),
    ]
    for key in roots["keys"]:
        verification.extend(["--standalone-authority-key", key])
    environment = {name: value for name, value in os.environ.items() if name != "GENT_NODE_BINARY"}
    with tempfile.TemporaryDirectory() as directory:
        staged = pathlib.Path(directory)
        verifier = pathlib.Path(os.environ["GENTD_VERIFIER"]).resolve()
        gentd = staged / verifier.name
        shutil.copy2(verifier, gentd)
        shutil.copytree(pathlib.Path(os.environ["NODE_RUNTIME_DIR"]).resolve(), staged / "runtime" / "node", symlinks=True)
        data_dir = staged / "data"
        data_dir.mkdir()
        subprocess.run([str(gentd), *verification, "--data-dir", str(data_dir)], env=environment, check=True)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        raise SystemExit(f"staged authority verification failed: {error}") from error
