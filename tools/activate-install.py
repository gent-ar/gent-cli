#!/usr/bin/env python3
"""Atomically install and select one verified, paired Gent runtime release."""

from __future__ import annotations

import argparse
import fcntl
import os
import shutil
import stat
import sys
import tempfile
from pathlib import Path


LAUNCHER = """#!/usr/bin/env sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../lib/gent" && pwd)
exec "$root/current/$(basename -- "$0")" "$@"
"""


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runtime_root", type=Path)
    parser.add_argument("release_name")
    parser.add_argument("--source-release", type=Path)
    parser.add_argument("--bin-dir", type=Path)
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--idle-data-dir", type=Path)
    parser.add_argument("--stage-only", action="store_true")
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


def fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def fsync_file(path: Path) -> None:
    with path.open("rb") as file:
        os.fsync(file.fileno())


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
    if Path(target).parent != Path("releases") or Path(target).name in {"", ".", ".."}:
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
    fsync_directory(runtime_root)
    return release


def identical(left: Path, right: Path) -> bool:
    if left.stat().st_size != right.stat().st_size:
        return False
    with left.open("rb") as left_file, right.open("rb") as right_file:
        while block := left_file.read(1024 * 1024):
            if block != right_file.read(len(block)):
                return False
    return True


def prepare_release(runtime_root: Path, release_name: str, source: Path) -> Path:
    require_directory(source)
    for name in ("gent", "gentd"):
        require_executable(source / name)
    releases = runtime_root / "releases"
    destination = releases / release_name
    if destination.exists() or destination.is_symlink():
        release = release_path(runtime_root, release_name)
        if all(identical(source / name, release / name) for name in ("gent", "gentd")):
            return release
        fail(f"existing {release_name} differs from the verified release")
    stage = Path(tempfile.mkdtemp(prefix=".gent-stage-", dir=releases))
    try:
        for name in ("gent", "gentd"):
            output = stage / name
            shutil.copyfile(source / name, output)
            output.chmod(0o755)
            fsync_file(output)
        fsync_directory(stage)
        os.rename(stage, destination)
        fsync_directory(releases)
    except BaseException:
        shutil.rmtree(stage, ignore_errors=True)
        raise
    return release_path(runtime_root, release_name)


def publish_launchers(bin_dir: Path) -> None:
    bin_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    require_directory(bin_dir)
    for name in ("gent", "gentd"):
        descriptor, temporary_name = tempfile.mkstemp(prefix=f".{name}-", dir=bin_dir)
        temporary = Path(temporary_name)
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8") as file:
                file.write(LAUNCHER)
                file.flush()
                os.fsync(file.fileno())
            temporary.chmod(0o755)
            os.replace(temporary, bin_dir / name)
        finally:
            try:
                temporary.unlink()
            except FileNotFoundError:
                pass
    fsync_directory(bin_dir)


def lock_install(runtime_root: Path):
    runtime_root.mkdir(mode=0o700, parents=True, exist_ok=True)
    require_directory(runtime_root)
    lock_path = runtime_root / ".install.lock"
    try:
        details = lock_path.lstat()
    except FileNotFoundError:
        pass
    else:
        if stat.S_ISLNK(details.st_mode) or not stat.S_ISREG(details.st_mode):
            fail("install lock must be a real file")
    descriptor = os.open(lock_path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    return os.fdopen(descriptor, "a+b")


def activate_while_idle(runtime_root: Path, release_name: str, data_dir: Path | None) -> Path:
    if data_dir is None:
        return activate(runtime_root, release_name)
    data_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    lock_path = data_dir / "gentd.lock"
    with lock_path.open("a+b") as lock:
        try:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise SystemExit(
                f"Gent activation refused: gentd is running for {data_dir}; stop it before updating"
            ) from error
        try:
            return activate(runtime_root, release_name)
        finally:
            fcntl.flock(lock.fileno(), fcntl.LOCK_UN)


def install(args: argparse.Namespace) -> Path:
    if args.source_release is None or args.bin_dir is None:
        fail("source release and bin directory are required for installation")
    with lock_install(args.runtime_root) as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        try:
            releases = args.runtime_root / "releases"
            releases.mkdir(mode=0o700, exist_ok=True)
            require_directory(releases)
            try:
                (args.runtime_root / "current").lstat()
            except FileNotFoundError:
                pass
            else:
                validate_current(args.runtime_root)
                if not args.force:
                    fail(f"Gent is already installed in {args.bin_dir}; pass --force to replace it")
            release = prepare_release(args.runtime_root, args.release_name, args.source_release)
            publish_launchers(args.bin_dir)
            if args.stage_only:
                return release
            return activate_while_idle(args.runtime_root, args.release_name, args.idle_data_dir)
        finally:
            fcntl.flock(lock.fileno(), fcntl.LOCK_UN)


def main() -> None:
    args = arguments()
    release = install(args) if args.source_release else activate_while_idle(
        args.runtime_root, args.release_name, args.idle_data_dir
    )
    print(f"{'staged' if args.stage_only else 'activated'} {release.name}")


if __name__ == "__main__":
    try:
        main()
    except OSError as error:
        raise SystemExit(f"Gent activation failed: {error}") from error
