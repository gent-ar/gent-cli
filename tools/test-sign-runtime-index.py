#!/usr/bin/env python3
"""Deterministic checks for the signed runtime-release index generator."""

from __future__ import annotations

import base64
import json
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
INDEXER = ROOT / "tools" / "sign-runtime-index.py"
RELEASE_SIGNER = ROOT / "tools" / "sign-runtime-release.py"


def private_key(seed: bytes) -> str:
    prefix = bytes.fromhex("302e020100300506032b657004220420")
    return "-----BEGIN PRIVATE KEY-----\n" + base64.b64encode(prefix + seed).decode() + "\n-----END PRIVATE KEY-----\n"


def run(script: Path, *args: str) -> None:
    subprocess.run([sys.executable, str(script), *args], check=True, capture_output=True, text=True)


def main() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        key = root / "private.pem"
        key.write_text(private_key(bytes(range(32))), encoding="ascii")
        archive = root / "archive.json"
        archive.write_text(json.dumps({"schemaVersion": 1, "version": "v1.2.3", "target": "fixture-target", "archive": {"name": "gent.tar.gz", "sha256": "a" * 64, "size": 1}}))
        release = root / "fixture.runtime-release.json"
        run(RELEASE_SIGNER, "--archive-manifest", str(archive), "--version", "v1.2.3", "--target", "fixture-target", "--key-id", "release-1", "--private-key", str(key), "--expires-at", "4102444800", "--out", str(release))
        index = root / "index.json"
        run(INDEXER, "--runtime-release", str(release), "--key-id", "release-1", "--private-key", str(key), "--expires-at", "4102444800", "--out", str(index))
        value = json.loads(index.read_text())
        offer = value["payload"]["offers"][0]
        assert offer["releaseTag"] == "v1.2.3"
        assert offer["manifestName"] == release.name
        assert len(offer["manifestDigestSha256"]) == 64
        duplicate = subprocess.run([sys.executable, str(INDEXER), "--runtime-release", str(release), "--runtime-release", str(release), "--key-id", "release-1", "--private-key", str(key), "--expires-at", "4102444800", "--out", str(root / "bad.json")], capture_output=True, text=True)
        assert duplicate.returncode != 0
    print("runtime index signing checks passed")


if __name__ == "__main__":
    main()
