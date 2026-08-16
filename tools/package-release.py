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


def archive_name(version: str, target: str, archive_format: str) -> str:
    return f"gent-{version}-{target}.{archive_format}"


def add_tar_file(archive: tarfile.TarFile, binary: Path, root: str, epoch: int) -> None:
    info = archive.gettarinfo(str(binary), arcname=f"{root}/{binary.name}")
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    info.mtime = epoch
    with binary.open("rb") as source:
        archive.addfile(info, source)


def write_tar(archive_path: Path, files: list[Path], root: str, epoch: int) -> None:
    with archive_path.open("wb") as output:
        with gzip.GzipFile(filename="", mode="wb", fileobj=output, mtime=epoch) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                for binary in files:
                    add_tar_file(archive, binary, root, epoch)


def write_zip(archive_path: Path, files: list[Path], root: str, epoch: int) -> None:
    timestamp = max(epoch, 315532800)
    date_time = tuple(__import__("time").gmtime(timestamp)[:6])
    with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for binary in files:
            info = zipfile.ZipInfo(f"{root}/{binary.name}", date_time=date_time)
            info.external_attr = (0o755 << 16)
            archive.writestr(info, binary.read_bytes(), compress_type=zipfile.ZIP_DEFLATED)


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
    }
    (out_dir / f"{name}.manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def main() -> None:
    args = arguments()
    files = binaries(args.target_dir, args.suffix)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    name = archive_name(args.version, args.target, args.format)
    archive = args.out_dir / name
    root = f"gent-{args.version}-{args.target}"
    if archive.exists():
        archive.unlink()
    if args.format == "tar.gz":
        write_tar(archive, files, root, source_date_epoch())
    else:
        write_zip(archive, files, root, source_date_epoch())
    write_metadata(args.out_dir, name, args.version, args.target, files)


if __name__ == "__main__":
    main()
