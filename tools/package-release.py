#!/usr/bin/env python3
"""Create one deterministic Gent binary archive and its checksum manifest."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
import tarfile
import zipfile
from pathlib import Path


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--format", choices=("tar.gz", "zip"), required=True)
    parser.add_argument("--suffix", default="")
    parser.add_argument("--node-runtime-dir", type=Path, required=True)
    parser.add_argument("--claurst-runtime-dir", type=Path, required=True)
    return parser.parse_args()


def source_date_epoch() -> int:
    value = os.environ.get("SOURCE_DATE_EPOCH", "0")
    try:
        return max(0, int(value))
    except ValueError as error:
        raise SystemExit("SOURCE_DATE_EPOCH must be an integer") from error


def binaries(target_dir: Path, suffix: str) -> list[Path]:
    paths = [target_dir / f"gent{suffix}", target_dir / f"gentd{suffix}"]
    if suffix == ".exe":
        paths.append(target_dir / "gent-launcher.exe")
    missing = [str(path) for path in paths if not path.is_file()]
    if missing:
        raise SystemExit(f"missing release binary: {', '.join(missing)}")
    return paths


def runtime_files(runtime_dir: Path, suffix: str) -> list[tuple[Path, str]]:
    node_name = f"node{suffix}"
    npm_name = "npm.cmd" if suffix == ".exe" else "npm"
    required = [
        runtime_dir / "bin" / node_name,
        runtime_dir / "bin" / npm_name,
        runtime_dir / "lib" / "node_modules" / "npm" / "bin" / "npm-cli.js",
    ]
    missing = [str(path) for path in required if not path.exists()]
    if missing:
        raise SystemExit(f"missing Node runtime file: {', '.join(missing)}")
    root = runtime_dir.resolve()
    files: list[tuple[Path, str]] = []
    for path in sorted(runtime_dir.rglob("*")):
        if not path.is_file() and not path.is_symlink():
            continue
        resolved = path.resolve()
        if not resolved.is_file() or root not in resolved.parents and resolved != root:
            raise SystemExit(f"Node runtime contains an unsafe file: {path}")
        files.append((resolved, path.relative_to(runtime_dir).as_posix()))
    return files


def claurst_files(runtime_dir: Path, suffix: str) -> list[tuple[Path, str]]:
    names = [f"claurst{suffix}", f"llama/llama-server{suffix}"]
    missing = [str(runtime_dir / name) for name in names if not (runtime_dir / name).is_file()]
    if missing:
        raise SystemExit(f"missing Claurst runtime file: {', '.join(missing)}")
    root = runtime_dir.resolve()
    files: list[tuple[Path, str]] = []
    for path in sorted(runtime_dir.rglob("*")):
        if not path.is_file():
            continue
        resolved = path.resolve()
        if not resolved.is_file() or (root not in resolved.parents and resolved != root):
            raise SystemExit(f"Claurst runtime contains an unsafe file: {path}")
        files.append((resolved, path.relative_to(runtime_dir).as_posix()))
    return files


def archive_name(version: str, target: str, archive_format: str) -> str:
    return f"gent-{version}-{target}.{archive_format}"


def add_tar_file(archive: tarfile.TarFile, source: Path, name: str, root: str, epoch: int) -> None:
    info = archive.gettarinfo(str(source), arcname=f"{root}/{name}")
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    info.mtime = epoch
    with source.open("rb") as input_file:
        archive.addfile(info, input_file)


def write_tar(archive_path: Path, files: list[tuple[Path, str]], root: str, epoch: int) -> None:
    with archive_path.open("wb") as output:
        with gzip.GzipFile(filename="", mode="wb", fileobj=output, mtime=epoch) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                for source, name in files:
                    add_tar_file(archive, source, name, root, epoch)


def write_zip(archive_path: Path, files: list[tuple[Path, str]], root: str, epoch: int) -> None:
    timestamp = max(epoch, 315532800)
    date_time = tuple(__import__("time").gmtime(timestamp)[:6])
    with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for source, name in files:
            info = zipfile.ZipInfo(f"{root}/{name}", date_time=date_time)
            info.external_attr = (0o755 << 16)
            archive.writestr(info, source.read_bytes(), compress_type=zipfile.ZIP_DEFLATED)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_metadata(out_dir: Path, name: str, version: str, target: str, files: list[Path]) -> None:
    archive = out_dir / name
    checksum = sha256(archive)
    (out_dir / f"{name}.sha256").write_text(f"{checksum}  {name}\n", encoding="utf-8")
    manifest = {
        "schemaVersion": 1,
        "version": version,
        "target": target,
        "archive": {"name": name, "sha256": checksum, "size": archive.stat().st_size},
        "binaries": [path.name for path in files],
        "capabilities": [
            "agent-chat-conversations-v1",
            "agent-chat-intents-v1",
            "agent-chat-transcript-v1",
            "agent-chat-turn-follow-v1",
            "agent-chat-permissions-v1",
            "attachments-v1",
            "local-models-v1",
        ],
        "runtimes": ["runtime/node", "runtime/claurst"],
    }
    (out_dir / f"{name}.manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def main() -> None:
    args = arguments()
    files = binaries(args.target_dir, args.suffix)
    runtime = runtime_files(args.node_runtime_dir, args.suffix)
    claurst = claurst_files(args.claurst_runtime_dir, args.suffix)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    name = archive_name(args.version, args.target, args.format)
    archive = args.out_dir / name
    root = f"gent-{args.version}-{args.target}"
    if archive.exists():
        archive.unlink()
    if args.format == "tar.gz":
        write_tar(archive, [(path, path.name) for path in files] + [(path, f"runtime/node/{name}") for path, name in runtime] + [(path, f"runtime/claurst/{name}") for path, name in claurst], root, source_date_epoch())
    else:
        write_zip(archive, [(path, path.name) for path in files] + [(path, f"runtime/node/{name}") for path, name in runtime] + [(path, f"runtime/claurst/{name}") for path, name in claurst], root, source_date_epoch())
    write_metadata(args.out_dir, name, args.version, args.target, files)


if __name__ == "__main__":
    main()
