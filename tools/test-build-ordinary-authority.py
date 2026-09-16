#!/usr/bin/env python3
from __future__ import annotations

import base64
import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from provider_pins import PINS, load_pins, require_pinned_authority


ROOT = Path(__file__).resolve().parents[1]
BUILDER = ROOT / "tools" / "build-ordinary-authority.py"
PKCS8_ED25519_PREFIX = bytes.fromhex("302e020100300506032b657004220420")
KEY_ID = "test-root-2026-08"
LIFETIME_SECONDS = 365 * 24 * 60 * 60


def signer():
    spec = importlib.util.spec_from_file_location("gent_release_signer", ROOT / "tools" / "sign-runtime-release.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


SIGNER = signer()
SEED = hashlib.sha256(b"gent-ordinary-authority-test-root").digest()
UNTRUSTED_SEED = hashlib.sha256(b"gent-ordinary-authority-untrusted-root").digest()
TARGETS = sorted(load_pins(PINS)["runtimes"]["node"]["artifacts"])


def private_key_pem(seed: bytes) -> str:
    body = base64.b64encode(PKCS8_ED25519_PREFIX + seed).decode("ascii")
    return f"-----BEGIN PRIVATE KEY-----\n{body}\n-----END PRIVATE KEY-----\n"


def measured_digest(target: str) -> str:
    return hashlib.sha256(target.encode("ascii")).hexdigest()


def complete_measurements() -> dict[str, dict[str, object]]:
    return {target: {"target": target, "node_sha256": measured_digest(target)} for target in TARGETS}


def build(directory: Path, measurements: dict[str, dict[str, object]], **environment: str | None) -> tuple[subprocess.CompletedProcess[str], Path]:
    digests = directory / "node-digests"
    digests.mkdir()
    for name, record in measurements.items():
        (digests / f"{name}.json").write_text(json.dumps(record), encoding="utf-8")
    roots = directory / "root-keys.json"
    roots.write_text(
        json.dumps({"version": 1, "keys": [f"{KEY_ID}:{SIGNER.public_key(SEED).hex()}"]}),
        encoding="utf-8",
    )
    out = directory / "ordinary-authority.json"
    values = {"RUNTIME_RELEASE_KEY_ID": KEY_ID, "RUNTIME_RELEASE_PRIVATE_KEY": private_key_pem(SEED), **environment}
    process_environment = {name: value for name, value in os.environ.items() if name not in values}
    process_environment.update({name: value for name, value in values.items() if value is not None})
    result = subprocess.run(
        [sys.executable, str(BUILDER), "--node-digests", str(digests), "--root-keys", str(roots), "--out", str(out)],
        capture_output=True,
        text=True,
        env=process_environment,
    )
    return result, out


def refusal(measurements: dict[str, dict[str, object]], **environment: str | None) -> str:
    with tempfile.TemporaryDirectory() as directory:
        result, out = build(Path(directory), measurements, **environment)
        assert result.returncode != 0, result.stdout
        assert not out.exists(), "a refused build still wrote an authority"
        return result.stderr


def decompress(encoded: bytes) -> tuple[int, int, int, int]:
    field = SIGNER.FIELD
    value = int.from_bytes(encoded, "little")
    sign, y = value >> 255, value & ((1 << 255) - 1)
    numerator = (y * y - 1) % field
    denominator = (SIGNER.CURVE_D * y * y + 1) % field
    square = numerator * pow(denominator, field - 2, field) % field
    x = pow(square, (field + 3) // 8, field)
    if x * x % field != square:
        x = x * pow(2, (field - 1) // 4, field) % field
    if x % 2 != sign:
        x = field - x
    return (x, y, 1, x * y % field)


def multiply(point: tuple[int, int, int, int], scalar: int) -> tuple[int, int, int, int]:
    result = (0, 1, 1, 0)
    while scalar:
        if scalar & 1:
            result = SIGNER.point_add(result, point)
        point = SIGNER.point_double(point)
        scalar >>= 1
    return result


def signature_verifies(public_key: bytes, message: bytes, signature: bytes) -> bool:
    commitment, scalar = signature[:32], int.from_bytes(signature[32:], "little")
    if scalar >= SIGNER.ORDER:
        return False
    challenge = int.from_bytes(hashlib.sha512(commitment + public_key + message).digest(), "little") % SIGNER.ORDER
    expected = SIGNER.point_add(decompress(commitment), multiply(decompress(public_key), challenge))
    return SIGNER.encode_point(SIGNER.scalar_multiply(scalar)) == SIGNER.encode_point(expected)


def test_signing_requires_a_named_root_key_and_its_private_half() -> None:
    for environment in (
        {"RUNTIME_RELEASE_KEY_ID": None},
        {"RUNTIME_RELEASE_PRIVATE_KEY": None},
        {"RUNTIME_RELEASE_KEY_ID": None, "RUNTIME_RELEASE_PRIVATE_KEY": None},
        {"RUNTIME_RELEASE_KEY_ID": "not a key id"},
        {"RUNTIME_RELEASE_PRIVATE_KEY": ""},
    ):
        message = refusal(complete_measurements(), **environment)
        assert "must name the ordinary authority root key" in message, message


def test_a_root_key_outside_the_committed_trust_file_cannot_sign() -> None:
    message = refusal(complete_measurements(), RUNTIME_RELEASE_PRIVATE_KEY=private_key_pem(UNTRUSTED_SEED))
    assert f"root key {KEY_ID} is not a committed trusted root" in message, message


def test_every_pinned_node_target_must_be_measured() -> None:
    measurements = complete_measurements()
    dropped = TARGETS[-1]
    del measurements[dropped]
    message = refusal(measurements)
    assert f"Node runtime measurements are missing for: {dropped}" in message, message
    assert "ordinary authority build failed" in message, message


def test_a_measurement_must_carry_a_sha256_node_digest() -> None:
    for digest in ("", "z" * 64, measured_digest(TARGETS[0])[:63], measured_digest(TARGETS[0]).upper()):
        measurements = complete_measurements()
        measurements[TARGETS[0]] = {"target": TARGETS[0], "node_sha256": digest}
        message = refusal(measurements)
        assert f"{TARGETS[0]}.json is not one measurement of a pinned Node runtime target" in message, message


def test_an_unknown_or_repeated_target_measurement_is_refused() -> None:
    unknown = complete_measurements()
    unknown["aarch64-unknown-freebsd"] = {"target": "aarch64-unknown-freebsd", "node_sha256": "a" * 64}
    message = refusal(unknown)
    assert "aarch64-unknown-freebsd.json is not one measurement" in message, message
    repeated = complete_measurements()
    repeated["zzz-second-report-of-the-same-target"] = {"target": TARGETS[0], "node_sha256": "b" * 64}
    message = refusal(repeated)
    assert "zzz-second-report-of-the-same-target.json is not one measurement" in message, message


def test_a_signed_authority_round_trips_the_pins_and_its_root_signature() -> None:
    with tempfile.TemporaryDirectory() as directory:
        before = int(time.time())
        result, out = build(Path(directory), complete_measurements())
        assert result.returncode == 0, result.stderr
        after = int(time.time())
        envelope = json.loads(out.read_text(encoding="utf-8"))
        assert envelope["key_id"] == KEY_ID
        payload = envelope["payload"]
        require_pinned_authority(payload, PINS)
        assert before + LIFETIME_SECONDS <= payload["expires_at_unix_seconds"] <= after + LIFETIME_SECONDS
        canonical = json.dumps(payload, separators=(",", ":"), ensure_ascii=True, sort_keys=True).encode("utf-8")
        roots = json.loads((Path(directory) / "root-keys.json").read_text(encoding="utf-8"))["keys"]
        public_key = bytes.fromhex(next(key.split(":", 1)[1] for key in roots if key.startswith(f"{KEY_ID}:")))
        assert signature_verifies(public_key, canonical, bytes.fromhex(envelope["signature_hex"]))
        assert not signature_verifies(public_key, canonical + b" ", bytes.fromhex(envelope["signature_hex"]))


def test_every_nested_document_is_signed_by_its_own_purpose_key() -> None:
    with tempfile.TemporaryDirectory() as directory:
        result, out = build(Path(directory), complete_measurements())
        assert result.returncode == 0, result.stderr
        payload = json.loads(out.read_text(encoding="utf-8"))["payload"]
        documents = [(payload["compatibility"], payload["compatibility_keys"]), (payload["package_policy"], payload["package_policy_keys"])]
        documents.extend((provider["evidence"], provider["evidence_keys"]) for provider in payload["providers"])
        for document, keys in documents:
            assert [key["key_id"] for key in keys] == [document["key_id"]], document["key_id"]
            message = json.dumps(document["payload"], separators=(",", ":"), ensure_ascii=False).encode("utf-8")
            public_key = bytes.fromhex(keys[0]["public_key_hex"])
            assert signature_verifies(public_key, message, bytes.fromhex(document["signature_hex"])), document["key_id"]
        assert {document["key_id"] for document, _ in documents} == {
            f"{KEY_ID}.compatibility",
            f"{KEY_ID}.package-policy",
            f"{KEY_ID}.evidence",
        }
        expiries = {document["payload"]["expires_at_unix_seconds"] for document, _ in documents}
        assert expiries == {payload["expires_at_unix_seconds"]}, expiries


def test_the_evidence_covers_the_exact_signed_compatibility_envelope() -> None:
    with tempfile.TemporaryDirectory() as directory:
        result, out = build(Path(directory), complete_measurements())
        assert result.returncode == 0, result.stderr
        payload = json.loads(out.read_text(encoding="utf-8"))["payload"]
        expected = hashlib.sha256(
            json.dumps(payload["compatibility"], separators=(",", ":"), ensure_ascii=False).encode("utf-8")
        ).hexdigest()
        for provider in payload["providers"]:
            evidence = provider["evidence"]["payload"]
            assert evidence["compatibility_manifest_sha256"] == expected, provider["provider"]
            assert evidence["provider"] == provider["provider"]
            assert "malformed_tolerance" not in evidence["scenarios"]
            assert len(evidence["scenarios"]) == 14, sorted(evidence["scenarios"])


if __name__ == "__main__":
    for name, test in sorted(globals().items()):
        if name.startswith("test_"):
            test()
    print("ordinary authority build checks passed")
