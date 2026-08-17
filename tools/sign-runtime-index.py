#!/usr/bin/env python3
"""Create a signed, expiring index of exact runtime-release metadata assets."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SIGNER = ROOT / "tools" / "sign-runtime-release.py"
CHANNELS = ("stable", "beta", "canary")


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime-release", type=Path, action="append", required=True)
    parser.add_argument("--key-id", required=True)
    parser.add_argument("--private-key", type=Path, required=True)
    parser.add_argument("--expires-at", type=int, required=True)
    parser.add_argument("--out", type=Path, required=True)
    return parser.parse_args()


def signer():
    spec = importlib.util.spec_from_file_location("runtime_release_signer", SIGNER)
    if spec is None or spec.loader is None:
        raise ValueError("runtime release signer is unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def release_offer(path: Path) -> dict[str, object]:
    if not path.is_file() or path.is_symlink() or "/" in path.name:
        raise ValueError("runtime release must be a real file with a safe asset name")
    envelope = json.loads(path.read_text(encoding="utf-8"))
    payload = envelope.get("payload")
    if not isinstance(payload, dict):
        raise ValueError("runtime release envelope has an unsupported shape")
    version, channel, artifact = (
        payload.get("releaseVersion"),
        payload.get("channel"),
        payload.get("artifact"),
    )
    if not isinstance(version, dict) or channel not in CHANNELS or not isinstance(artifact, dict):
        raise ValueError("runtime release envelope has an unsupported shape")
    numbers = tuple(version.get(name) for name in ("major", "minor", "patch"))
    target = artifact.get("target")
    if (
        any(not isinstance(number, int) or number < 0 for number in numbers)
        or not isinstance(target, str)
        or not target
    ):
        raise ValueError("runtime release offer is invalid")
    return {
        "releaseTag": f"v{numbers[0]}.{numbers[1]}.{numbers[2]}",
        "releaseVersion": dict(zip(("major", "minor", "patch"), numbers)),
        "channel": channel,
        "target": target,
        "manifestName": path.name,
        "manifestDigestSha256": sha256(path),
    }


def payload(args: argparse.Namespace) -> dict[str, object]:
    if not args.key_id or args.expires_at <= 0:
        raise ValueError("key id and expiration are required")
    offers = [release_offer(path) for path in args.runtime_release]
    offers.sort(key=lambda value: (value["channel"], value["target"]))
    identities = {(offer["channel"], offer["target"]) for offer in offers}
    if len(offers) != len(identities):
        raise ValueError("runtime index repeats a channel and target offer")
    return {
        "indexVersion": 1,
        "expiresAtUnixSeconds": args.expires_at,
        "revoked": False,
        "offers": offers,
    }


def main() -> None:
    args = arguments()
    module = signer()
    body = payload(args)
    envelope = {
        "keyId": args.key_id,
        "payload": body,
        "signatureHex": module.sign(args.private_key, module.canonical(body)),
    }
    module.atomic_write(args.out, module.canonical(envelope) + b"\n")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise SystemExit(f"runtime index signing failed: {error}") from error
