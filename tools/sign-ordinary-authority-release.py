#!/usr/bin/env python3
"""Sign one canonical Gent ordinary-authority release payload.

The caller owns the Ed25519 private key. This tool never generates, stores, or
copies it into the artifact; it only writes the signed public envelope consumed
by the uncomposed Gent daemon authority loader.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
KEY_ID = re.compile(r"[A-Za-z0-9._-]{1,128}$")
FIELDS = (
    "version",
    "expires_at_unix_seconds",
    "revoked",
    "compatibility",
    "compatibility_keys",
    "package_policy",
    "package_policy_keys",
    "providers",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--payload", type=Path, required=True)
    parser.add_argument("--key-id", required=True)
    parser.add_argument("--private-key", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    return parser.parse_args()


def signer():
    path = ROOT / "tools" / "sign-runtime-release.py"
    spec = importlib.util.spec_from_file_location("gent_release_signer", path)
    if spec is None or spec.loader is None:
        raise ValueError("shared Ed25519 signer is unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_payload(path: Path) -> dict[str, object]:
    if not path.is_file() or path.is_symlink():
        raise ValueError("payload must be a real readable file")
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict) or set(value) != set(FIELDS):
        raise ValueError("payload has an unsupported authority-release shape")
    if value["version"] != 1 or value["revoked"] is not False:
        raise ValueError("payload must be an active ordinary-authority release v1")
    if not isinstance(value["expires_at_unix_seconds"], int) or value["expires_at_unix_seconds"] <= 0:
        raise ValueError("payload expiry is invalid")
    for field in ("compatibility_keys", "package_policy_keys", "providers"):
        if not isinstance(value[field], list) or not value[field]:
            raise ValueError(f"payload {field} must be a nonempty array")
    return {field: value[field] for field in FIELDS}


def main() -> None:
    args = parse_args()
    if KEY_ID.fullmatch(args.key_id) is None:
        raise ValueError("key id is invalid")
    payload = load_payload(args.payload)
    module = signer()
    canonical = json.dumps(payload, separators=(",", ":"), ensure_ascii=True, sort_keys=True).encode("utf-8")
    envelope = {
        "key_id": args.key_id,
        "payload": payload,
        "signature_hex": module.sign(args.private_key, canonical),
    }
    module.atomic_write(args.out, json.dumps(envelope, separators=(",", ":"), ensure_ascii=True).encode("utf-8") + b"\n")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise SystemExit(f"ordinary authority release signing failed: {error}") from error
