#!/usr/bin/env python3
"""Strictly verify one Gent package archive and its signed-release metadata."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import stat
import tarfile
import zipfile
from pathlib import Path
from typing import Any


SHA256 = re.compile(r"[0-9a-f]{64}\Z")
SAFE_COMPONENT = re.compile(r"[A-Za-z0-9._+-]+\Z")


def fail(message: str) -> None:
    raise ValueError(message)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def required_string(value: Any, name: str) -> str:
    if not isinstance(value, str) or not value:
        fail(f"{name} must be a non-empty string")
    return value


def safe_component(value: str, name: str) -> str:
    if not SAFE_COMPONENT.fullmatch(value):
        fail(f"{name} contains unsafe characters")
    return value


def expected_binaries(archive: Path) -> list[str]:
    if archive.name.endswith(".tar.gz"):
        return ["gent", "gentd"]
    if archive.suffix == ".zip":
        return ["gent.exe", "gentd.exe", "gent-launcher.exe"]
    fail("archive format must be .tar.gz or .zip")


def read_manifest(path: Path, archive: Path) -> tuple[str, str, str, int, list[str]]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("manifest is not valid UTF-8 JSON") from error
    if not isinstance(value, dict) or set(value) != {"schemaVersion", "version", "target", "archive", "binaries"}:
        fail("manifest has an unexpected schema")
    if value["schemaVersion"] != 1 or isinstance(value["schemaVersion"], bool):
        fail("manifest schemaVersion must be 1")
    version = safe_component(required_string(value["version"], "manifest version"), "manifest version")
    target = safe_component(required_string(value["target"], "manifest target"), "manifest target")
    bundle = value["archive"]
    if not isinstance(bundle, dict) or set(bundle) != {"name", "sha256", "size"}:
        fail("manifest archive has an unexpected schema")
    name = required_string(bundle["name"], "archive name")
    digest = required_string(bundle["sha256"], "archive sha256")
    size = bundle["size"]
    if name != archive.name or not SHA256.fullmatch(digest):
        fail("manifest archive name or sha256 is invalid")
    if not isinstance(size, int) or isinstance(size, bool) or size < 0:
        fail("manifest archive size is invalid")
    binaries = value["binaries"]
    if binaries != expected_binaries(archive):
        fail("manifest binaries do not match archive format")
    return version, target, digest, size, binaries


def verify_tar(archive: Path, root: str, binaries: list[str]) -> None:
    expected = [f"{root}/{binary}" for binary in binaries]
    try:
        with tarfile.open(archive, "r:gz") as bundle:
            members = bundle.getmembers()
            names = [member.name for member in members]
            if names != expected or len(set(names)) != len(names):
                fail("tar archive has unexpected or duplicate members")
            for member in members:
                if not member.isfile() or member.linkname:
                    fail("tar archive contains a non-regular member")
                source = bundle.extractfile(member)
                if source is None:
                    fail("tar archive member cannot be read")
                while source.read(1024 * 1024):
                    pass
    except (OSError, tarfile.TarError) as error:
        raise ValueError("tar archive is invalid") from error


def verify_zip(archive: Path, root: str, binaries: list[str]) -> None:
    expected = [f"{root}/{binary}" for binary in binaries]
    try:
        with zipfile.ZipFile(archive) as bundle:
            entries = bundle.infolist()
            names = [entry.filename for entry in entries]
            if names != expected or len(set(names)) != len(names):
                fail("zip archive has unexpected or duplicate members")
            for entry in entries:
                kind = stat.S_IFMT(entry.external_attr >> 16)
                if entry.is_dir() or entry.flag_bits & 1 or kind not in (0, stat.S_IFREG):
                    fail("zip archive contains a non-regular member")
            if bundle.testzip() is not None:
                fail("zip archive has an invalid member checksum")
    except (OSError, zipfile.BadZipFile) as error:
        raise ValueError("zip archive is invalid") from error


def verify(archive: Path, manifest_path: Path, checksum_path: Path, version: str | None, target: str | None) -> None:
    if not archive.is_file() or archive.is_symlink():
        fail("archive must be a regular file")
    manifest_version, manifest_target, digest, size, binaries = read_manifest(manifest_path, archive)
    if version is not None and manifest_version != version:
        fail("manifest version does not match expected version")
    if target is not None and manifest_target != target:
        fail("manifest target does not match expected target")
    extension = ".tar.gz" if archive.name.endswith(".tar.gz") else ".zip"
    if archive.name != f"gent-{manifest_version}-{manifest_target}{extension}":
        fail("archive name does not match manifest version and target")
    if archive.stat().st_size != size or sha256(archive) != digest:
        fail("archive size or sha256 does not match manifest")
    expected_checksum = f"{digest}  {archive.name}".encode()
    if checksum_path.read_bytes() not in (expected_checksum + b"\n", expected_checksum + b"\r\n"):
        fail("checksum file does not match manifest")
    root = f"gent-{manifest_version}-{manifest_target}"
    if extension == ".tar.gz":
        verify_tar(archive, root, binaries)
    else:
        verify_zip(archive, root, binaries)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--checksum", type=Path, required=True)
    parser.add_argument("--version", help="expected signed release version")
    parser.add_argument("--target", help="expected signed release target")
    args = parser.parse_args()
    try:
        verify(args.archive, args.manifest, args.checksum, args.version, args.target)
    except (OSError, ValueError) as error:
        raise SystemExit(f"release archive verification failed: {error}") from error
    print(f"verified {args.archive.name} ({sha256(args.archive)})")


if __name__ == "__main__":
    main()
