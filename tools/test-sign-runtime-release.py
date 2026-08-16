#!/usr/bin/env python3
"""Deterministic checks for the signed runtime-release manifest generator."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SIGNER = ROOT / "tools/sign-runtime-release.py"


def run(*args: str) -> None:
    subprocess.run([sys.executable, str(SIGNER), *args], check=True, capture_output=True, text=True)


def verify(public: Path, envelope: Path, payload: Path, signature: Path) -> bool:
    value = json.loads(envelope.read_text())
    payload.write_bytes(json.dumps(value["payload"], separators=(",", ":")).encode())
    signature.write_bytes(bytes.fromhex(value["signatureHex"]))
    return subprocess.run(
        ["openssl", "pkeyutl", "-verify", "-rawin", "-pubin", "-inkey", str(public), "-in", str(payload), "-sigfile", str(signature)],
        capture_output=True,
        text=True,
    ).returncode == 0


def main() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        private, public = root / "private.pem", root / "public.pem"
        subprocess.run(["openssl", "genpkey", "-algorithm", "ED25519", "-out", str(private)], check=True)
        subprocess.run(["openssl", "pkey", "-in", str(private), "-pubout", "-out", str(public)], check=True)
        archive = root / "archive.manifest.json"
        archive.write_text(json.dumps({"schemaVersion": 1, "version": "v1.2.3", "target": "fixture-target", "archive": {"name": "gent.tar.gz", "sha256": "a" * 64, "size": 1}}))
        envelope = root / "runtime-release.json"
        run("--archive-manifest", str(archive), "--version", "v1.2.3", "--target", "fixture-target", "--key-id", "release-1", "--private-key", str(private), "--expires-at", "4102444800", "--out", str(envelope))
        payload, signature = root / "payload.json", root / "signature.bin"
        assert verify(public, envelope, payload, signature)
        value = json.loads(envelope.read_text())
        assert value["payload"]["artifact"]["digestSha256"] == "a" * 64
        value["payload"]["rolloutPercent"] = 99
        envelope.write_text(json.dumps(value))
        assert not verify(public, envelope, payload, signature)
        bad = subprocess.run([sys.executable, str(SIGNER), "--archive-manifest", str(archive), "--version", "bad", "--target", "fixture-target", "--key-id", "release-1", "--private-key", str(private), "--expires-at", "1", "--out", str(root / "bad.json")], capture_output=True, text=True)
        assert bad.returncode != 0
    print("runtime release signing checks passed")


if __name__ == "__main__":
    main()
