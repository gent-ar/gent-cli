#!/usr/bin/env python3
"""Atomically select one verified, paired Gent runtime release directory."""

from __future__ import annotations

import argparse
import os
import stat
import sys
from pathlib import Path


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runtime_root", type=Path)
    parser.add_argument("release_name")
    return parser.parse_args()


def fail(message: str) -> None:
    raise SystemExit(f"Gent activation refused: {message}")


def lstat(path: Path) -> os.stat_result:
    try:
        return path.lstat()
    except FileNotFoundError:
        fail(f"missing {path}")


def require_directory(path: Path) -> None:
    details = lstat(path)
    if stat.S_ISLNK(details.st_mode) or not stat.S_ISDIR(details.st_mode):
        fail(f"{path} must be a real directory")


def require_executable(path: Path) -> None:
    details = lstat(path)
    if stat.S_ISLNK(details.st_mode) or not stat.S_ISREG(details.st_mode):
        fail(f"{path} must be a real file")
    if details.st_mode & 0o111 == 0:
        fail(f"{path} is not executable")


def release_path(runtime_root: Path, release_name: str) -> Path:
    if not release_name or release_name in {".", ".."}:
        fail("release name is required")
    candidate = Path(release_name)
    if candidate.name != release_name or candidate.is_absolute():
        fail("release name is not a single path component")
    releases = runtime_root / "releases"
    require_directory(releases)
    release = releases / release_name
    require_directory(release)
    require_executable(release / "gent")
    require_executable(release / "gentd")
    return release


def validate_current(runtime_root: Path) -> None:
    current = runtime_root / "current"
    try:
        details = current.lstat()
    except FileNotFoundError:
        return
    if not stat.S_ISLNK(details.st_mode):
        fail("current exists but is not a symlink")
    target = os.readlink(current)
    expected_parent = Path(target).parent
    if expected_parent != Path("releases") or Path(target).name in {"", ".", ".."}:
        fail("current does not point to a managed release")


def activate(runtime_root: Path, release_name: str) -> Path:
    require_directory(runtime_root)
    release = release_path(runtime_root, release_name)
    validate_current(runtime_root)
    current = runtime_root / "current"
    temporary = runtime_root / f".current-{os.getpid()}"
    try:
        temporary.unlink()
    except FileNotFoundError:
        pass
    os.symlink(Path("releases") / release.name, temporary)
    os.replace(temporary, current)
    return release


def main() -> None:
    args = arguments()
    release = activate(args.runtime_root, args.release_name)
    print(f"activated {release.name}")


if __name__ == "__main__":
    try:
        main()
    except OSError as error:
        raise SystemExit(f"Gent activation failed: {error}") from error
