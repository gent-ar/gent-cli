#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import re
import time
from pathlib import Path

from authority_evidence import provider_evidence
from authority_keys import nested_signing_keys
from provider_pins import PINS, load_pins, pinned_entries, require_pinned_authority


ROOT = Path(__file__).resolve().parents[1]
ROOT_KEYS = ROOT / "platform" / "authority" / "root-keys.json"
CORPUS = ROOT / "fixtures" / "public-driver-transcripts"
TERMS_VERSION = "terms-1"
LIFETIME_SECONDS = 365 * 24 * 60 * 60
KEY_ID = re.compile(r"[A-Za-z0-9._-]{1,96}")
SHA256 = re.compile(r"[0-9a-f]{64}")
PRIVATE_KEY_ENV = "RUNTIME_RELEASE_PRIVATE_KEY"
KEY_ID_ENV = "RUNTIME_RELEASE_KEY_ID"


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Build and sign the ordinary provider authority from pins, measured Node runtimes and recorded evidence")
    parser.add_argument("--node-digests", type=Path, required=True)
    parser.add_argument("--root-keys", type=Path, default=ROOT_KEYS)
    parser.add_argument("--out", type=Path, required=True)
    return parser.parse_args()


def signer():
    spec = importlib.util.spec_from_file_location("gent_release_signer", ROOT / "tools" / "sign-runtime-release.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


SIGNER = signer()


def compact(value: object) -> bytes:
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def canonical(value: object) -> bytes:
    return json.dumps(value, separators=(",", ":"), ensure_ascii=True, sort_keys=True).encode("utf-8")


def signed(key: tuple[str, bytes], payload: dict[str, object]) -> dict[str, object]:
    return {"key_id": key[0], "payload": payload, "signature_hex": SIGNER.sign_seed(key[1], compact(payload))}


def public(key: tuple[str, bytes]) -> list[dict[str, str]]:
    return [{"key_id": key[0], "public_key_hex": SIGNER.public_key(key[1]).hex()}]


def measured_node_digests(pins: dict[str, object], directory: Path) -> dict[str, str]:
    artifacts = pins["runtimes"]["node"]["artifacts"]
    measured = {}
    for path in sorted(directory.glob("*.json")):
        record = json.loads(path.read_text(encoding="utf-8"))
        target, digest = record["target"], record["node_sha256"]
        if target not in artifacts or target in measured or SHA256.fullmatch(digest) is None:
            raise ValueError(f"{path.name} is not one measurement of a pinned Node runtime target")
        measured[target] = digest
    missing = sorted(set(artifacts) - set(measured))
    if missing:
        raise ValueError("Node runtime measurements are missing for: " + ", ".join(missing))
    return {artifacts[target]["provider_target"]: digest for target, digest in measured.items()}


def root_signing_key(roots_path: Path) -> tuple[str, bytes]:
    key_id = os.environ.get(KEY_ID_ENV, "")
    private_key = os.environ.get(PRIVATE_KEY_ENV, "")
    if KEY_ID.fullmatch(key_id) is None or not private_key:
        raise ValueError(f"{KEY_ID_ENV} and {PRIVATE_KEY_ENV} must name the ordinary authority root key")
    seed = SIGNER.parse_seed(private_key.encode("ascii"))
    trusted = json.loads(roots_path.read_text(encoding="utf-8"))["keys"]
    if f"{key_id}:{SIGNER.public_key(seed).hex()}" not in trusted:
        raise ValueError(f"root key {key_id} is not a committed trusted root in {roots_path}")
    return key_id, seed


def authority_payload(root: tuple[str, bytes], pins: dict[str, object], node_digests: dict[str, str], expires_at: int) -> dict[str, object]:
    nested = nested_signing_keys(*root)
    entries = pinned_entries(pins, node_digests, TERMS_VERSION)
    compatibility = signed(nested["compatibility"], {"manifest_version": 1, "expires_at_unix_seconds": expires_at, "entries": entries["compatibility"]})
    compatibility_sha256 = hashlib.sha256(compact(compatibility)).hexdigest()
    return {
        "version": 1,
        "expires_at_unix_seconds": expires_at,
        "revoked": False,
        "compatibility": compatibility,
        "compatibility_keys": public(nested["compatibility"]),
        "package_policy": signed(nested["package-policy"], {"policy_version": 1, "expires_at_unix_seconds": expires_at, "entries": entries["package_policy"]}),
        "package_policy_keys": public(nested["package-policy"]),
        "providers": [
            {
                "provider": provider,
                "evidence": signed(nested["evidence"], provider_evidence(CORPUS, provider, compatibility_sha256, expires_at)),
                "evidence_keys": public(nested["evidence"]),
            }
            for provider in ("claude", "codex")
        ],
    }


def main() -> None:
    args = arguments()
    pins = load_pins(PINS)
    root = root_signing_key(args.root_keys)
    payload = authority_payload(root, pins, measured_node_digests(pins, args.node_digests), int(time.time()) + LIFETIME_SECONDS)
    require_pinned_authority(payload, PINS)
    envelope = {"key_id": root[0], "payload": payload, "signature_hex": SIGNER.sign_seed(root[1], canonical(payload))}
    SIGNER.atomic_write(args.out, json.dumps(envelope, separators=(",", ":"), ensure_ascii=True).encode("utf-8") + b"\n")
    print(f"ordinary authority signed by {root[0]}: sha256 {hashlib.sha256(args.out.read_bytes()).hexdigest()}, expires {payload['expires_at_unix_seconds']}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        raise SystemExit(f"ordinary authority build failed: {error}") from error
