#!/usr/bin/env python3
"""Externally stage, health-check, and atomically activate a Gent runtime pair."""

from __future__ import annotations

import argparse
import fcntl
import os
import subprocess
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def activator() -> Path:
    staged = Path(__file__).with_name("activate-install.py")
    if staged.is_file() and not staged.is_symlink():
        return staged
    return ROOT / "tools/activate-install.py"


def args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime-root", type=Path, required=True)
    parser.add_argument("--release-name", required=True)
    parser.add_argument("--source-release", type=Path, required=True)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--data-dir", type=Path, required=True)
    parser.add_argument("--timeout-seconds", type=int, default=30)
    return parser.parse_args()


def run(arguments: list[str]) -> None:
    subprocess.run(arguments, check=True, capture_output=True, text=True)


def current(root: Path) -> str:
    link = root / "current"
    target = os.readlink(link)
    if Path(target).parent != Path("releases"):
        raise ValueError("current pointer is not a managed release")
    return Path(target).name


def activate(values: argparse.Namespace, stage_only: bool = False, release: str | None = None) -> None:
    command = ["python3", str(activator()), str(values.runtime_root), release or values.release_name]
    if stage_only:
        command.extend(("--source-release", str(values.source_release), "--source-supervisor", str(Path(__file__)), "--bin-dir", str(values.bin_dir), "--force", "--stage-only"))
    else:
        command.extend(("--idle-data-dir", str(values.data_dir)))
    run(command)


def lock_is_free(data_dir: Path) -> bool:
    data_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    with (data_dir / "gentd.lock").open("a+b") as lock:
        try:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            return True
        except BlockingIOError:
            return False


def wait_for_lock(data_dir: Path, deadline: float) -> None:
    while time.monotonic() < deadline:
        if lock_is_free(data_dir):
            return
        time.sleep(0.05)
    raise TimeoutError("old gentd did not drain before activation")


def health(gent: Path, gentd: Path, deadline: float) -> None:
    with tempfile.TemporaryDirectory(prefix="gent-runtime-health-") as temporary:
        data = Path(temporary)
        process = subprocess.Popen([str(gentd), "--data-dir", str(data)], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        try:
            while time.monotonic() < deadline:
                if (data / "gentd.sock").exists():
                    run([str(gent), "--data-dir", str(data), "--no-autostart", "status"])
                    return
                if process.poll() is not None:
                    raise RuntimeError(f"staged gentd exited before its health handshake: {process.stderr.read().strip()}")
                time.sleep(0.05)
            raise TimeoutError("staged gentd did not expose local IPC")
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    process.kill()


def main() -> None:
    values = args()
    if not 1 <= values.timeout_seconds <= 120:
        raise ValueError("timeout must be 1..120 seconds")
    previous = current(values.runtime_root)
    deadline = time.monotonic() + values.timeout_seconds
    activate(values, stage_only=True)
    staged = values.runtime_root / "releases" / values.release_name
    health(staged / "gent", staged / "gentd", deadline)
    wait_for_lock(values.data_dir, deadline)
    try:
        activate(values)
        health(values.bin_dir / "gent", values.bin_dir / "gentd", deadline)
    except BaseException:
        if lock_is_free(values.data_dir):
            activate(values, release=previous)
        raise
    print(f"activated {values.release_name}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, TimeoutError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit(f"runtime activation supervisor failed: {error}") from error
