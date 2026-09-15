#!/usr/bin/env python3
import argparse
import json
import os
import pathlib
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
    command = [
        str(pathlib.Path(os.environ["GENTD_VERIFIER"]).resolve()),
        "--standalone-authority",
        "--verify-standalone-authority-release",
        "--standalone-authority-release",
        str(release),
    ]
    for key in roots["keys"]:
        command.extend(["--standalone-authority-key", key])
    environment = os.environ.copy()
    environment["GENT_NODE_BINARY"] = str(pathlib.Path(os.environ["NODE_BINARY"]).resolve())
    with tempfile.TemporaryDirectory() as data_dir:
        command.extend(["--data-dir", data_dir])
        subprocess.run(command, env=environment, check=True)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        raise SystemExit(f"staged authority verification failed: {error}") from error
