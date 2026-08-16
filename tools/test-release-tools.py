#!/usr/bin/env python3
"""Portable deterministic tests for Gent release packaging and verification."""

from __future__ import annotations

import hashlib
import io
import json
import os
import stat
import subprocess
import sys
import tarfile
import tempfile
import warnings
import zipfile
from pathlib import Path
from typing import Any, Callable


ROOT = Path(__file__).resolve().parent.parent
PACKAGER = ROOT / "tools/package-release.py"
VERIFIER = ROOT / "tools/verify-release.py"


def package(target: Path, output: Path, archive_format: str, suffix: str = "") -> Path:
    environment = {**os.environ, "SOURCE_DATE_EPOCH": "1700000000"}
    subprocess.run(
        [sys.executable, str(PACKAGER), "--target-dir", str(target), "--out-dir", str(output),
         "--version", "0.1.0", "--target", "fixture-target", "--format", archive_format,
         "--suffix", suffix], check=True, env=environment)
    return output / f"gent-0.1.0-fixture-target.{archive_format}"


def command(archive: Path, *expected: str) -> list[str]:
    return [sys.executable, str(VERIFIER), str(archive), "--manifest", f"{archive}.manifest.json",
            "--checksum", f"{archive}.sha256", *expected]


def verify(archive: Path) -> None:
    subprocess.run(command(archive, "--version", "0.1.0", "--target", "fixture-target"), check=True)


def rejects(archive: Path, *expected: str) -> None:
    result = subprocess.run(command(archive, *expected), capture_output=True, text=True)
    assert result.returncode != 0
    assert "verification failed" in result.stderr


def rewrite_metadata(archive: Path, mutate: Callable[[dict[str, Any]], None] | None = None) -> None:
    manifest_path = Path(f"{archive}.manifest.json")
    manifest = json.loads(manifest_path.read_text())
    if mutate is not None:
        mutate(manifest)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    manifest["archive"] = {"name": archive.name, "sha256": digest, "size": archive.stat().st_size}
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
    Path(f"{archive}.sha256").write_text(f"{digest}  {archive.name}\n", encoding="utf-8")


def replace_zip(archive: Path, entries: list[tuple[str, bytes, int]]) -> None:
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", UserWarning)
        with zipfile.ZipFile(archive, "w") as bundle:
            for name, content, mode in entries:
                info = zipfile.ZipInfo(name)
                info.external_attr = mode << 16
                bundle.writestr(info, content)
    rewrite_metadata(archive)


def replace_tar_with_symlink(archive: Path) -> None:
    root = "gent-0.1.0-fixture-target"
    with tarfile.open(archive, "w:gz") as bundle:
        link = tarfile.TarInfo(f"{root}/gent")
        link.type = tarfile.SYMTYPE
        link.linkname = "outside"
        bundle.addfile(link)
        binary = tarfile.TarInfo(f"{root}/gentd")
        binary.size = 1
        bundle.addfile(binary, io.BytesIO(b"x"))
    rewrite_metadata(archive)


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
        rejects(first, "--version", "0.2.0")
        second.write_bytes(second.read_bytes() + b"tampered")
        rejects(second)
        tar_symlink = package(target, root / "tar-symlink", "tar.gz")
        replace_tar_with_symlink(tar_symlink)
        rejects(tar_symlink)
        (target / "gent.exe").write_bytes(b"gent windows fixture\n")
        (target / "gentd.exe").write_bytes(b"gentd windows fixture\n")
        (target / "gent-launcher.exe").write_bytes(b"launcher windows fixture\n")
        windows = package(target, root / "windows", "zip", ".exe")
        verify(windows)
        assert json.loads(Path(f"{windows}.manifest.json").read_text())["binaries"] == [
            "gent.exe", "gentd.exe", "gent-launcher.exe"
        ]
        root_name = "gent-0.1.0-fixture-target"
        duplicate = package(target, root / "zip-duplicate", "zip", ".exe")
        replace_zip(duplicate, [(f"{root_name}/gent.exe", b"a", stat.S_IFREG | 0o755),
                                (f"{root_name}/gent.exe", b"b", stat.S_IFREG | 0o755),
                                (f"{root_name}/gentd.exe", b"c", stat.S_IFREG | 0o755),
                                (f"{root_name}/gent-launcher.exe", b"d", stat.S_IFREG | 0o755)])
        rejects(duplicate)
        zip_link = package(target, root / "zip-link", "zip", ".exe")
        replace_zip(zip_link, [(f"{root_name}/gent.exe", b"a", stat.S_IFLNK | 0o777),
                               (f"{root_name}/gentd.exe", b"b", stat.S_IFREG | 0o755),
                               (f"{root_name}/gent-launcher.exe", b"c", stat.S_IFREG | 0o755)])
        rejects(zip_link)
        bad_binaries = package(target, root / "bad-binaries", "zip", ".exe")
        rewrite_metadata(bad_binaries, lambda manifest: manifest.update(binaries=["gent", "gentd"]))
        rejects(bad_binaries)
    print("release tooling checks passed")


if __name__ == "__main__":
    main()
