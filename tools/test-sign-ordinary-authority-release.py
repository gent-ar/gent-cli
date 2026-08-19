#!/usr/bin/env python3
"""Contract checks for the ordinary-authority release signer."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SIGNER = ROOT / "tools" / "sign-ordinary-authority-release.py"


def payload() -> dict[str, object]:
    return {
        "version": 1,
        "expires_at_unix_seconds": 100,
        "revoked": False,
        "compatibility": {"placeholder": True},
        "compatibility_keys": [{"placeholder": True}],
        "package_policy": {"placeholder": True},
        "package_policy_keys": [{"placeholder": True}],
        "providers": [{"placeholder": True}],
    }


def run(*arguments: object) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, SIGNER, *map(str, arguments)],
        check=False,
        capture_output=True,
        text=True,
    )


def test_signs_only_the_payload_with_the_requested_public_key_id() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source, key, output = root / "payload.json", root / "key", root / "release.json"
        source.write_text(json.dumps(payload()), encoding="utf-8")
        key.write_bytes(bytes(range(32)))
        result = run("--payload", source, "--key-id", "ordinary-1", "--private-key", key, "--out", output)
        assert result.returncode == 0, result.stderr
        envelope = json.loads(output.read_text(encoding="utf-8"))
        assert envelope["key_id"] == "ordinary-1"
        assert envelope["payload"] == payload()
        assert len(envelope["signature_hex"]) == 128
        assert key.read_bytes() == bytes(range(32))


def test_rejects_revoked_or_unknown_payload_shape() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source, key, output = root / "payload.json", root / "key", root / "release.json"
        invalid = payload()
        invalid["revoked"] = True
        invalid["unknown"] = True
        source.write_text(json.dumps(invalid), encoding="utf-8")
        key.write_bytes(bytes(range(32)))
        result = run("--payload", source, "--key-id", "ordinary-1", "--private-key", key, "--out", output)
        assert result.returncode != 0
        assert not output.exists()


if __name__ == "__main__":
    test_signs_only_the_payload_with_the_requested_public_key_id()
    test_rejects_revoked_or_unknown_payload_shape()
    print("ordinary authority release signer checks passed")
