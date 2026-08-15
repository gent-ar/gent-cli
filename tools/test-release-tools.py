#!/usr/bin/env python3
"""Portable deterministic tests for release package and verification tools."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
PACKAGER = ROOT / "tools/package-release.py"
VERIFIER = ROOT / "tools/verify-release.py"


def package(target: Path, output: Path, archive_format: str, suffix: str = "") -> Path:
    environment = {**os.environ, "SOURCE_DATE_EPOCH": "1700000000"}
    subprocess.run(
        [
            sys.executable, str(PACKAGER), "--target-dir", str(target), "--out-dir", str(output),
            "--version", "0.1.0", "--target", "fixture-target", "--format", archive_format,
            "--suffix", suffix,
        ],
        check=True,
        env=environment,
    )
    return output / f"gent-0.1.0-fixture-target.{archive_format}"


def verify(archive: Path) -> None:
    subprocess.run(
        [
            sys.executable, str(VERIFIER), str(archive), "--manifest", f"{archive}.manifest.json",
            "--checksum", f"{archive}.sha256",
        ],
        check=True,
    )


def rejects_tampered_archive(archive: Path) -> None:
    archive.write_bytes(archive.read_bytes() + b"tampered")
    result = subprocess.run(
        [
            sys.executable, str(VERIFIER), str(archive), "--manifest", f"{archive}.manifest.json",
            "--checksum", f"{archive}.sha256",
        ],
        capture_output=True,
        text=True,
    )
    assert result.returncode != 0
    assert "verification failed" in result.stderr


def main() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        target = root / "target"
        target.mkdir()
        (target / "gent").write_bytes(b"gent fixture\n")
        (target / "gentd").write_bytes(b"gentd fixture\n")
        first = package(target, root / "first", "tar.gz")
        second = package(target, root / "second", "tar.gz")
        assert first.read_bytes() == second.read_bytes()
        verify(first)
        manifest = json.loads(Path(f"{first}.manifest.json").read_text())
        assert manifest["binaries"] == ["gent", "gentd"]
        rejects_tampered_archive(second)
        (target / "gent.exe").write_bytes(b"gent windows fixture\n")
        (target / "gentd.exe").write_bytes(b"gentd windows fixture\n")
        windows = package(target, root / "windows", "zip", ".exe")
        verify(windows)
    print("release tooling checks passed")


if __name__ == "__main__":
    main()
