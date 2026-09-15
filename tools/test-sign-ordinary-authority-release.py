#!/usr/bin/env python3
"""Contract checks for the ordinary-authority release signer."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

from provider_pins_testing import pinned_payload, write_pins


ROOT = Path(__file__).resolve().parents[1]
SIGNER = ROOT / "tools" / "sign-ordinary-authority-release.py"


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
        source.write_text(json.dumps(pinned_payload()), encoding="utf-8")
        key.write_bytes(bytes(range(32)))
        pins = write_pins(root)
        result = run("--payload", source, "--key-id", "ordinary-1", "--private-key", key, "--out", output, "--pins", pins)
        assert result.returncode == 0, result.stderr
        envelope = json.loads(output.read_text(encoding="utf-8"))
        assert envelope["key_id"] == "ordinary-1"
        assert envelope["payload"] == pinned_payload()
        assert len(envelope["signature_hex"]) == 128
        assert key.read_bytes() == bytes(range(32))


def test_rejects_revoked_or_unknown_payload_shape() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source, key, output = root / "payload.json", root / "key", root / "release.json"
        invalid = pinned_payload()
        invalid["revoked"] = True
        invalid["unknown"] = True
        source.write_text(json.dumps(invalid), encoding="utf-8")
        key.write_bytes(bytes(range(32)))
        result = run("--payload", source, "--key-id", "ordinary-1", "--private-key", key, "--out", output, "--pins", write_pins(root))
        assert result.returncode != 0
        assert not output.exists()


def test_refuses_a_payload_that_differs_from_the_provider_pins() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source, key, output = root / "payload.json", root / "key", root / "release.json"
        drifted = pinned_payload()
        drifted["compatibility"]["payload"]["entries"][1]["version"] = "codex-cli 9.9.9"
        source.write_text(json.dumps(drifted), encoding="utf-8")
        key.write_bytes(bytes(range(32)))
        result = run("--payload", source, "--key-id", "ordinary-1", "--private-key", key, "--out", output, "--pins", write_pins(root))
        assert result.returncode != 0
        assert "compatibility codex-9.2.0-darwin-arm64 'codex-cli 9.9.9'" in result.stderr, result.stderr
        assert not output.exists()


def test_refuses_a_compatibility_digest_that_is_not_the_pinned_native_binary() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source, key, output = root / "payload.json", root / "key", root / "release.json"
        drifted = pinned_payload()
        drifted["compatibility"]["payload"]["entries"][1]["digest_sha256"] = "f" * 64
        source.write_text(json.dumps(drifted), encoding="utf-8")
        key.write_bytes(bytes(range(32)))
        result = run("--payload", source, "--key-id", "ordinary-1", "--private-key", key, "--out", output, "--pins", write_pins(root))
        assert result.returncode != 0
        assert "compatibility codex-9.2.0-darwin-arm64 'codex-cli 9.2.0' 'ffff" in result.stderr, result.stderr
        assert not output.exists()


def test_refuses_pinned_versions_without_contract_snapshots() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source, key, output = root / "payload.json", root / "key", root / "release.json"
        source.write_text(json.dumps(pinned_payload()), encoding="utf-8")
        key.write_bytes(bytes(range(32)))
        result = run("--payload", source, "--key-id", "ordinary-1", "--private-key", key, "--out", output, "--pins", write_pins(root, snapshots=False))
        assert result.returncode != 0
        assert "has no fixtures/provider-contracts/claude/9.1.0/source.json snapshot" in result.stderr, result.stderr
        assert not output.exists()


if __name__ == "__main__":
    test_signs_only_the_payload_with_the_requested_public_key_id()
    test_rejects_revoked_or_unknown_payload_shape()
    test_refuses_a_payload_that_differs_from_the_provider_pins()
    test_refuses_a_compatibility_digest_that_is_not_the_pinned_native_binary()
    test_refuses_pinned_versions_without_contract_snapshots()
    print("ordinary authority release signer checks passed")
