#!/usr/bin/env python3
"""Create an Ed25519 PKCS#8 key for Gent runtime-release metadata signing.

The private PEM is intentionally written only to a caller-selected new file.
Store it in the GitHub Actions secret `GENT_RUNTIME_RELEASE_PRIVATE_KEY`; copy
the printed public key and key id into the matching protected repository vars.
"""

from __future__ import annotations

import argparse
import base64
import importlib.util
import os
import secrets
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SIGNER = ROOT / "tools" / "sign-runtime-release.py"
PREFIX = bytes.fromhex("302e020100300506032b657004220420")


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--key-id", required=True)
    parser.add_argument("--private-key-out", type=Path, required=True)
    return parser.parse_args()


def signer_module():
    spec = importlib.util.spec_from_file_location("runtime_release_signer", SIGNER)
    if spec is None or spec.loader is None:
        raise ValueError("runtime-release signer is unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def pem(seed: bytes) -> bytes:
    encoded = base64.b64encode(PREFIX + seed).decode("ascii")
    return f"-----BEGIN PRIVATE KEY-----\n{encoded}\n-----END PRIVATE KEY-----\n".encode("ascii")


def write_new(path: Path, contents: bytes) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(contents)
            output.flush()
            os.fsync(output.fileno())
    except BaseException:
        path.unlink(missing_ok=True)
        raise


def main() -> None:
    values = arguments()
    if not values.key_id.strip() or any(value.isspace() for value in values.key_id):
        raise ValueError("key id must be nonempty and contain no whitespace")
    if values.private_key_out.is_absolute() and values.private_key_out == Path("/"):
        raise ValueError("private key output must name a file")
    seed = secrets.token_bytes(32)
    module = signer_module()
    write_new(values.private_key_out, pem(seed))
    print(f"GENT_RUNTIME_RELEASE_KEY_ID={values.key_id}")
    print(f"GENT_RUNTIME_RELEASE_PUBLIC_KEY={module.public_key(seed).hex()}")
    print(f"private_key_file={values.private_key_out}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError) as error:
        raise SystemExit(f"runtime release key generation failed: {error}") from error
