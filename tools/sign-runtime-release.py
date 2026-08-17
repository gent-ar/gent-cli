#!/usr/bin/env python3
"""Create a canonical Ed25519-signed Gent runtime-release manifest.

The private key is caller-owned and never written to the release artifact. The
result is distinct from Sigstore archive signatures: it is the compact metadata
envelope the daemon revalidates before it can stage an update.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import re
import tempfile
from pathlib import Path


VERSION = re.compile(r"v?(\d+)\.(\d+)\.(\d+)$")
CHANNELS = ("stable", "beta", "canary")
FIELD = 2**255 - 19
ORDER = 2**252 + 27742317777372353535851937790883648493
CURVE_D = (-121665 * pow(121666, FIELD - 2, FIELD)) % FIELD
BASE_X = 15112221349535400772501151409588531511454012693041857206046113283949847762202
BASE_Y = 46316835694926478169428394003475163141307993866256225615783033603165251855960
BASE = (BASE_X, BASE_Y, 1, BASE_X * BASE_Y % FIELD)
PKCS8_ED25519_PREFIX = bytes.fromhex("302e020100300506032b657004220420")


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
    parser.add_argument("--schema-max", type=int, default=23)
    parser.add_argument("--minimum-app-version", default="0.1.5")
    parser.add_argument("--channel", choices=CHANNELS, default="stable")
    parser.add_argument("--rollout-percent", type=int, default=100)
    parser.add_argument("--forward-only-schema", action="store_true")
    parser.add_argument("--out", type=Path, required=True)
    return parser.parse_args()


def version(value: str) -> dict[str, int]:
    match = VERSION.fullmatch(value)
    if match is None:
        raise ValueError("version must be vMAJOR.MINOR.PATCH")
    return dict(zip(("major", "minor", "patch"), map(int, match.groups())))


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


def load_seed(key: Path) -> bytes:
    if not key.is_file() or key.is_symlink():
        raise ValueError("private key must be a real readable file")
    value = key.read_bytes()
    if len(value) == 32:
        return value
    try:
        lines = value.decode("ascii").splitlines()
        body = "".join(line for line in lines if not line.startswith("---"))
        der = base64.b64decode(body, validate=True)
    except (UnicodeDecodeError, ValueError) as error:
        raise ValueError("private key must be a raw Ed25519 seed or PKCS#8 PEM") from error
    if len(der) != 48 or not der.startswith(PKCS8_ED25519_PREFIX):
        raise ValueError("private key must be an Ed25519 PKCS#8 PEM")
    return der[len(PKCS8_ED25519_PREFIX) :]


def point_add(left: tuple[int, int, int, int], right: tuple[int, int, int, int]) -> tuple[int, int, int, int]:
    x1, y1, z1, t1 = left
    x2, y2, z2, t2 = right
    a, b = (y1 - x1) * (y2 - x2), (y1 + x1) * (y2 + x2)
    c, d = 2 * CURVE_D * t1 * t2, 2 * z1 * z2
    e, f, g, h = b - a, d - c, d + c, b + a
    return (e * f % FIELD, g * h % FIELD, f * g % FIELD, e * h % FIELD)


def point_double(point: tuple[int, int, int, int]) -> tuple[int, int, int, int]:
    x, y, z, _ = point
    a, b, c, d = x * x, y * y, 2 * z * z, -(x * x)
    e, g, f, h = (x + y) * (x + y) - a - b, d + b, d + b - c, d - b
    return (e * f % FIELD, g * h % FIELD, f * g % FIELD, e * h % FIELD)


def scalar_multiply(scalar: int) -> tuple[int, int, int, int]:
    result, point = (0, 1, 1, 0), BASE
    while scalar:
        if scalar & 1:
            result = point_add(result, point)
        point = point_double(point)
        scalar >>= 1
    return result


def encode_point(point: tuple[int, int, int, int]) -> bytes:
    x, y, z, _ = point
    inverse = pow(z, FIELD - 2, FIELD)
    x, y = x * inverse % FIELD, y * inverse % FIELD
    encoded = bytearray(y.to_bytes(32, "little"))
    encoded[31] |= (x & 1) << 7
    return bytes(encoded)


def public_key(seed: bytes) -> bytes:
    """Derive the raw Ed25519 verifying key for a 32-byte signing seed."""
    digest = hashlib.sha512(seed).digest()
    scalar = int.from_bytes(digest[:32], "little")
    scalar &= (1 << 254) - 8
    scalar |= 1 << 254
    return encode_point(scalar_multiply(scalar))


def sign(key: Path, content: bytes) -> str:
    seed = load_seed(key)
    digest = hashlib.sha512(seed).digest()
    scalar = int.from_bytes(digest[:32], "little")
    scalar &= (1 << 254) - 8
    scalar |= 1 << 254
    public = public_key(seed)
    nonce = int.from_bytes(hashlib.sha512(digest[32:] + content).digest(), "little") % ORDER
    encoded_nonce = encode_point(scalar_multiply(nonce))
    challenge = int.from_bytes(hashlib.sha512(encoded_nonce + public + content).digest(), "little") % ORDER
    return (encoded_nonce + ((nonce + challenge * scalar) % ORDER).to_bytes(32, "little")).hex()


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
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise SystemExit(f"runtime release signing failed: {error}") from error
