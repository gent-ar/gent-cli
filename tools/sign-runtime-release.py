#!/usr/bin/env python3
"""Create a canonical Ed25519-signed Gent runtime-release manifest.

The private key is caller-owned and never written to the release artifact. The
result is distinct from Sigstore archive signatures: it is the compact metadata
envelope the daemon revalidates before it can stage an update.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import tempfile
from pathlib import Path


VERSION = re.compile(r"v?(\d+)\.(\d+)\.(\d+)$")
CHANNELS = ("stable", "beta", "canary")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive-manifest", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--key-id", required=True)
    parser.add_argument("--private-key", type=Path, required=True)
    parser.add_argument("--expires-at", type=int, required=True)
    parser.add_argument("--protocol-min", type=int, default=1)
    parser.add_argument("--protocol-max", type=int, default=1)
    parser.add_argument("--schema-min", type=int, default=1)
    parser.add_argument("--schema-max", type=int, default=22)
    parser.add_argument("--minimum-app-version", default="0.1.4")
    parser.add_argument("--channel", choices=CHANNELS, default="stable")
    parser.add_argument("--rollout-percent", type=int, default=100)
    parser.add_argument("--forward-only-schema", action="store_true")
    parser.add_argument("--out", type=Path, required=True)
    return parser.parse_args()


def version(value: str) -> dict[str, int]:
    match = VERSION.fullmatch(value)
    if match is None:
        raise ValueError("version must be vMAJOR.MINOR.PATCH")
    return dict(zip(("major", "minor", "patch"), map(int, match.groups()), strict=True))


def load_archive(path: Path, release: str, target: str) -> dict[str, object]:
    value = json.loads(path.read_text(encoding="utf-8"))
    archive = value.get("archive")
    if not isinstance(archive, dict) or value.get("schemaVersion") != 1:
        raise ValueError("archive manifest has an unsupported shape")
    if value.get("version") != release or value.get("target") != target:
        raise ValueError("archive manifest version or target does not match the signed release")
    name, digest, size = archive.get("name"), archive.get("sha256"), archive.get("size")
    if not isinstance(name, str) or not isinstance(digest, str) or not isinstance(size, int):
        raise ValueError("archive manifest lacks a typed archive identity")
    if not re.fullmatch(r"[0-9a-f]{64}", digest) or size <= 0:
        raise ValueError("archive manifest digest or size is invalid")
    return {"target": target, "archiveName": name, "digestSha256": digest, "sizeBytes": size}


def payload(args: argparse.Namespace) -> dict[str, object]:
    release = version(args.version)
    minimum_app = version(args.minimum_app_version)
    if not args.key_id or args.expires_at <= 0 or not 0 <= args.rollout_percent <= 100:
        raise ValueError("key id, expiration, and rollout percentage are invalid")
    if min(args.protocol_min, args.protocol_max, args.schema_min, args.schema_max) < 1:
        raise ValueError("compatibility minima must be positive")
    if args.protocol_min > args.protocol_max or args.schema_min > args.schema_max:
        raise ValueError("compatibility ranges are invalid")
    return {
        "manifestVersion": 1,
        "releaseVersion": release,
        "protocolMin": args.protocol_min,
        "protocolMax": args.protocol_max,
        "schemaMin": args.schema_min,
        "schemaMax": args.schema_max,
        "minimumAppVersion": minimum_app,
        "channel": args.channel,
        "rolloutPercent": args.rollout_percent,
        "expiresAtUnixSeconds": args.expires_at,
        "revoked": False,
        "forwardOnlySchema": args.forward_only_schema,
        "artifact": load_archive(args.archive_manifest, args.version, args.target),
    }


def canonical(value: dict[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), ensure_ascii=True).encode("utf-8")


def sign(key: Path, content: bytes) -> str:
    if not key.is_file() or key.is_symlink():
        raise ValueError("private key must be a real readable file")
    with tempfile.TemporaryDirectory(prefix="gent-runtime-sign-") as directory:
        root = Path(directory)
        input_path, signature_path = root / "payload.json", root / "signature.bin"
        input_path.write_bytes(content)
        subprocess.run(
            ["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", str(key), "-in", str(input_path), "-out", str(signature_path)],
            check=True,
            capture_output=True,
            text=True,
        )
        signature = signature_path.read_bytes()
    if len(signature) != 64:
        raise ValueError("runtime release key must produce a 64-byte Ed25519 signature")
    return signature.hex()


def atomic_write(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def main() -> None:
    args = parse_args()
    body = payload(args)
    envelope = {"keyId": args.key_id, "payload": body, "signatureHex": sign(args.private_key, canonical(body))}
    atomic_write(args.out, canonical(envelope) + b"\n")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        raise SystemExit(f"runtime release signing failed: {error}") from error
