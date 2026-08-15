#!/usr/bin/env python3
"""Verify a Gent archive against its adjacent JSON manifest and SHA-256 file."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--checksum", type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    checksum_line = args.checksum.read_text(encoding="utf-8").strip()
    expected = manifest.get("archive", {}).get("sha256")
    actual = sha256(args.archive)
    required = f"{expected}  {args.archive.name}"
    if (
        manifest.get("schemaVersion") != 1
        or manifest.get("archive", {}).get("name") != args.archive.name
        or not isinstance(expected, str)
        or len(expected) != 64
        or checksum_line != required
        or actual != expected
    ):
        raise SystemExit("release archive verification failed")
    print(f"verified {args.archive.name} ({actual})")


if __name__ == "__main__":
    main()
