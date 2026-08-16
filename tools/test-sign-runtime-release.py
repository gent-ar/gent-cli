#!/usr/bin/env python3
"""Deterministic checks for the signed runtime-release manifest generator."""

from __future__ import annotations

import json
import base64
import sys
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SIGNER = ROOT / "tools/sign-runtime-release.py"


def run(*args: str) -> None:
    subprocess.run([sys.executable, str(SIGNER), *args], check=True, capture_output=True, text=True)


def private_key(seed: bytes) -> str:
    der = bytes.fromhex("302e020100300506032b657004220420") + seed
    return "-----BEGIN PRIVATE KEY-----\n" + base64.b64encode(der).decode() + "\n-----END PRIVATE KEY-----\n"


def main() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        private = root / "private.pem"
        private.write_text(private_key(bytes(range(32))), encoding="ascii")
        archive = root / "archive.manifest.json"
        archive.write_text(json.dumps({"schemaVersion": 1, "version": "v1.2.3", "target": "fixture-target", "archive": {"name": "gent.tar.gz", "sha256": "a" * 64, "size": 1}}))
        envelope = root / "runtime-release.json"
        run("--archive-manifest", str(archive), "--version", "v1.2.3", "--target", "fixture-target", "--key-id", "release-1", "--private-key", str(private), "--expires-at", "4102444800", "--out", str(envelope))
        value = json.loads(envelope.read_text())
        assert value["payload"]["artifact"]["digestSha256"] == "a" * 64
        assert len(value["signatureHex"]) == 128
        repeat = root / "repeat.json"
        run("--archive-manifest", str(archive), "--version", "v1.2.3", "--target", "fixture-target", "--key-id", "release-1", "--private-key", str(private), "--expires-at", "4102444800", "--out", str(repeat))
        assert envelope.read_bytes() == repeat.read_bytes()
        value["payload"]["rolloutPercent"] = 99
        envelope.write_text(json.dumps(value))
        bad = subprocess.run([sys.executable, str(SIGNER), "--archive-manifest", str(archive), "--version", "bad", "--target", "fixture-target", "--key-id", "release-1", "--private-key", str(private), "--expires-at", "1", "--out", str(root / "bad.json")], capture_output=True, text=True)
        assert bad.returncode != 0
    print("runtime release signing checks passed")


if __name__ == "__main__":
    main()
