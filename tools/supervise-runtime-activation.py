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


def activator(values: argparse.Namespace) -> Path:
    if values.activator is not None:
        if values.activator.is_file() and not values.activator.is_symlink():
            return values.activator
        raise ValueError("activation helper must be a real file")
    for name in ("activate-install.py", "gent-activate-install.py"):
        staged = Path(__file__).with_name(name)
        if staged.is_file() and not staged.is_symlink():
            return staged
    return ROOT / "tools/activate-install.py"


def args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime-root", type=Path, required=True)
    parser.add_argument("--release-name", required=True)
    parser.add_argument("--source-release", type=Path, required=True)
    parser.add_argument("--source-auto-updater", type=Path)
    parser.add_argument("--source-update-material", type=Path)
    parser.add_argument("--activator", type=Path)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--data-dir", type=Path, required=True)
    parser.add_argument("--recover-attempt-id")
    parser.add_argument("--runtime-release-cache", type=Path)
    parser.add_argument("--runtime-release-trust", type=Path)
    parser.add_argument("--runtime-release-key", action="append", default=[])
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
    command = ["python3", str(activator(values)), str(values.runtime_root), release or values.release_name]
    if stage_only:
        command.extend(("--source-release", str(values.source_release), "--source-supervisor", str(Path(__file__)), "--bin-dir", str(values.bin_dir), "--force", "--stage-only"))
        if values.source_auto_updater is not None:
            command.extend(("--source-auto-updater", str(values.source_auto_updater)))
        if values.source_update_material is not None:
            command.extend(("--source-update-material", str(values.source_update_material)))
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


def recovery_command(values: argparse.Namespace) -> list[str] | None:
    if values.recover_attempt_id is None:
        return None
    if values.runtime_release_cache is None or (
        values.runtime_release_trust is None and not values.runtime_release_key
    ):
        raise ValueError("successor recovery requires a release cache and trust input")
    command = [
        str(values.bin_dir / "gentd"), "--data-dir", str(values.data_dir),
        "--runtime-update-recover-authority", "--runtime-update-attempt-id",
        values.recover_attempt_id, "--runtime-release-cache", str(values.runtime_release_cache),
    ]
    if values.runtime_release_trust is not None:
        command.extend(("--runtime-release-trust", str(values.runtime_release_trust)))
    for key in values.runtime_release_key:
        command.extend(("--runtime-release-key", key))
    return command


def recover(values: argparse.Namespace, deadline: float) -> None:
    command = recovery_command(values)
    if command is None:
        health(values.bin_dir / "gent", values.bin_dir / "gentd", deadline)
        return
    process = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    last_probe_error = "local IPC is not ready"
    while time.monotonic() < deadline:
        if (values.data_dir / "gentd.sock").exists():
            try:
                run([str(values.bin_dir / "gent"), "--data-dir", str(values.data_dir), "--no-autostart", "status"])
                return
            except subprocess.CalledProcessError as error:
                last_probe_error = error.stderr.strip() or "local status probe failed"
        if process.poll() is not None:
            raise RuntimeError(f"successor exited before recovery handshake: {process.stderr.read().strip()}")
        time.sleep(0.05)
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()
    raise TimeoutError(f"successor did not complete a local IPC probe: {last_probe_error}")


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
        recover(values, deadline)
    except BaseException:
        if values.recover_attempt_id is None and lock_is_free(values.data_dir):
            activate(values, release=previous)
        raise
    print(f"activated {values.release_name}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, TimeoutError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit(f"runtime activation supervisor failed: {error}") from error
