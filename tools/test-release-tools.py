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


def package(target: Path, output: Path, runtime: Path, claurst: Path, archive_format: str, suffix: str = "") -> Path:
    environment = {**os.environ, "SOURCE_DATE_EPOCH": "1700000000"}
    subprocess.run(
        [sys.executable, str(PACKAGER), "--target-dir", str(target), "--out-dir", str(output),
         "--version", "0.1.0", "--target", "fixture-target", "--format", archive_format,
         "--suffix", suffix, "--node-runtime-dir", str(runtime), "--claurst-runtime-dir", str(claurst)], check=True, env=environment)
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


def replace_tar_with_traversal(archive: Path) -> None:
    root = "gent-0.1.0-fixture-target"
    required = [
        "gent",
        "gentd",
        "runtime/node/bin/node",
        "runtime/node/bin/npm",
        "runtime/node/lib/node_modules/npm/bin/npm-cli.js",
        "runtime/claurst/claurst",
        "runtime/claurst/llama/llama-server",
    ]
    with tarfile.open(archive, "w:gz") as bundle:
        for relative in [*required, "runtime/node/../outside"]:
            member = tarfile.TarInfo(f"{root}/{relative}")
            member.size = 1
            bundle.addfile(member, io.BytesIO(b"x"))
    rewrite_metadata(archive)


def main() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        target = root / "target"
        target.mkdir()
        (target / "gent").write_bytes(b"gent fixture\n")
        (target / "gentd").write_bytes(b"gentd fixture\n")
        (target / "gent").chmod(0o755)
        (target / "gentd").chmod(0o755)
        runtime = root / "runtime"
        (runtime / "bin").mkdir(parents=True)
        (runtime / "bin" / "node").write_bytes(b"node fixture\n")
        (runtime / "bin" / "npm").write_bytes(b"npm fixture\n")
        (runtime / "bin" / "node").chmod(0o755)
        (runtime / "bin" / "npm").chmod(0o755)
        npm_cli = runtime / "lib/node_modules/npm/bin/npm-cli.js"
        npm_cli.parent.mkdir(parents=True)
        npm_cli.write_bytes(b"npm cli fixture\n")
        claurst = root / "claurst"
        claurst.mkdir()
        (claurst / "claurst").write_bytes(b"claurst fixture\n")
        (claurst / "claurst").chmod(0o755)
        (claurst / "llama").mkdir()
        (claurst / "llama" / "llama-server").write_bytes(b"llama fixture\n")
        (claurst / "llama" / "llama-server").chmod(0o755)
        (claurst / "llama" / "libllama.so").write_bytes(b"llama library fixture\n")
        first = package(target, root / "first", runtime, claurst, "tar.gz")
        second = package(target, root / "second", runtime, claurst, "tar.gz")
        assert first.read_bytes() == second.read_bytes()
        verify(first)
        manifest = json.loads(Path(f"{first}.manifest.json").read_text())
        assert manifest["binaries"] == ["gent", "gentd"]
        assert "local-models-v1" in manifest["capabilities"]
        assert "workspace-git-v1" in manifest["capabilities"]
        assert "agent-chat-conversation-config-v1" in manifest["capabilities"]
        assert "agent-chat-checkpoint-v1" in manifest["capabilities"]
        assert "agent-chat-side-question-v1" in manifest["capabilities"]
        assert "permission-policy-v1" in manifest["capabilities"]
        assert "prompt-provider-provision-v1" in manifest["capabilities"]
        assert manifest["runtimes"] == ["runtime/node", "runtime/claurst"]
        rejects(first, "--version", "0.2.0")
        second.write_bytes(second.read_bytes() + b"tampered")
        rejects(second)
        tar_symlink = package(target, root / "tar-symlink", runtime, claurst, "tar.gz")
        replace_tar_with_symlink(tar_symlink)
        rejects(tar_symlink)
        tar_traversal = package(target, root / "tar-traversal", runtime, claurst, "tar.gz")
        replace_tar_with_traversal(tar_traversal)
        rejects(tar_traversal)
        (target / "gent.exe").write_bytes(b"gent windows fixture\n")
        (target / "gentd.exe").write_bytes(b"gentd windows fixture\n")
        (target / "gent-launcher.exe").write_bytes(b"launcher windows fixture\n")
        windows_runtime = root / "windows-runtime"
        (windows_runtime / "bin").mkdir(parents=True)
        (windows_runtime / "bin" / "node.exe").write_bytes(b"node fixture\n")
        (windows_runtime / "bin" / "npm.cmd").write_bytes(b"npm fixture\n")
        windows_npm_cli = windows_runtime / "lib/node_modules/npm/bin/npm-cli.js"
        windows_npm_cli.parent.mkdir(parents=True)
        windows_npm_cli.write_bytes(b"npm cli fixture\n")
        windows_claurst = root / "windows-claurst"
        windows_claurst.mkdir()
        (windows_claurst / "claurst.exe").write_bytes(b"claurst fixture\n")
        (windows_claurst / "llama").mkdir()
        (windows_claurst / "llama" / "llama-server.exe").write_bytes(b"llama fixture\n")
        (windows_claurst / "llama" / "llama.dll").write_bytes(b"llama library fixture\n")
        windows = package(target, root / "windows", windows_runtime, windows_claurst, "zip", ".exe")
        verify(windows)
        assert json.loads(Path(f"{windows}.manifest.json").read_text())["binaries"] == [
            "gent.exe", "gentd.exe", "gent-launcher.exe"
        ]
        root_name = "gent-0.1.0-fixture-target"
        duplicate = package(target, root / "zip-duplicate", windows_runtime, windows_claurst, "zip", ".exe")
        replace_zip(duplicate, [(f"{root_name}/gent.exe", b"a", stat.S_IFREG | 0o755),
                                (f"{root_name}/gent.exe", b"b", stat.S_IFREG | 0o755),
                                (f"{root_name}/gentd.exe", b"c", stat.S_IFREG | 0o755),
                                (f"{root_name}/gent-launcher.exe", b"d", stat.S_IFREG | 0o755)])
        rejects(duplicate)
        zip_link = package(target, root / "zip-link", windows_runtime, windows_claurst, "zip", ".exe")
        replace_zip(zip_link, [(f"{root_name}/gent.exe", b"a", stat.S_IFLNK | 0o777),
                               (f"{root_name}/gentd.exe", b"b", stat.S_IFREG | 0o755),
                               (f"{root_name}/gent-launcher.exe", b"c", stat.S_IFREG | 0o755)])
        rejects(zip_link)
        zip_traversal = package(target, root / "zip-traversal", windows_runtime, windows_claurst, "zip", ".exe")
        replace_zip(zip_traversal, [
            (f"{root_name}/gent.exe", b"a", stat.S_IFREG | 0o755),
            (f"{root_name}/gentd.exe", b"b", stat.S_IFREG | 0o755),
            (f"{root_name}/gent-launcher.exe", b"c", stat.S_IFREG | 0o755),
            (f"{root_name}/runtime/node/bin/node.exe", b"d", stat.S_IFREG | 0o755),
            (f"{root_name}/runtime/node/bin/npm.cmd", b"e", stat.S_IFREG | 0o755),
            (f"{root_name}/runtime/node/lib/node_modules/npm/bin/npm-cli.js", b"f", stat.S_IFREG | 0o755),
            (f"{root_name}/runtime/claurst/claurst.exe", b"g", stat.S_IFREG | 0o755),
            (f"{root_name}/runtime/claurst/llama/llama-server.exe", b"h", stat.S_IFREG | 0o755),
            (f"{root_name}/runtime/node/../outside", b"i", stat.S_IFREG | 0o755),
        ])
        rejects(zip_traversal)
        bad_binaries = package(target, root / "bad-binaries", windows_runtime, windows_claurst, "zip", ".exe")
        rewrite_metadata(bad_binaries, lambda manifest: manifest.update(binaries=["gent", "gentd"]))
        rejects(bad_binaries)
    print("release tooling checks passed")


if __name__ == "__main__":
    main()
