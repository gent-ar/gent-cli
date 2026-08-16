#!/usr/bin/env python3
"""Offline tests for paired Gent runtime activation."""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import fcntl
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
ACTIVATOR = ROOT / "tools" / "activate-install.py"


def release(root: Path, name: str, *, gentd: bool = True) -> Path:
    directory = root / "releases" / name
    directory.mkdir(parents=True)
    for binary in ("gent", "gentd"):
        if binary == "gentd" and not gentd:
            continue
        path = directory / binary
        path.write_text(f"#!/bin/sh\necho {binary}\n", encoding="utf-8")
        path.chmod(0o700)
    return directory


def activate(root: Path, name: str, data_dir: Path | None = None) -> subprocess.CompletedProcess[str]:
    arguments = [sys.executable, str(ACTIVATOR), str(root), name]
    if data_dir is not None:
        arguments.extend(("--idle-data-dir", str(data_dir)))
    return subprocess.run(
        arguments,
        capture_output=True,
        text=True,
    )


def current(root: Path) -> str:
    return os.readlink(root / "current")


def test_first_activation_and_atomic_replacement() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary) / "gent"
        root.mkdir()
        release(root, "v1-target")
        assert activate(root, "v1-target").returncode == 0
        assert current(root) == "releases/v1-target"
        release(root, "v2-target")
        assert activate(root, "v2-target").returncode == 0
        assert current(root) == "releases/v2-target"
        assert not list(root.glob(".current-*"))


def test_rejects_incomplete_release_without_changing_current() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary) / "gent"
        root.mkdir()
        release(root, "v1-target")
        assert activate(root, "v1-target").returncode == 0
        release(root, "v2-target", gentd=False)
        result = activate(root, "v2-target")
        assert result.returncode != 0
        assert "missing" in result.stderr
        assert current(root) == "releases/v1-target"


def test_rejects_unsafe_current_or_release_paths() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary) / "gent"
        root.mkdir()
        release(root, "v1-target")
        (root / "current").write_text("not a pointer", encoding="utf-8")
        assert activate(root, "v1-target").returncode != 0
        assert not (root / "releases" / "../escape").exists()
        assert activate(root, "../escape").returncode != 0


def test_rejects_symlinked_release_binary() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary) / "gent"
        root.mkdir()
        directory = release(root, "v1-target")
        (directory / "gentd").unlink()
        (directory / "gentd").symlink_to("gent")
        result = activate(root, "v1-target")
        assert result.returncode != 0
        assert "real file" in result.stderr
        assert not (root / "current").exists()


def test_idle_lock_refuses_activation_and_preserves_current_pair() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary) / "gent"
        data = Path(temporary) / "data"
        root.mkdir()
        data.mkdir()
        release(root, "v1-target")
        release(root, "v2-target")
        assert activate(root, "v1-target").returncode == 0
        with (data / "gentd.lock").open("a+b") as lock:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = activate(root, "v2-target", data)
            assert result.returncode != 0
            assert "gentd is running" in result.stderr
            assert current(root) == "releases/v1-target"
        assert activate(root, "v2-target", data).returncode == 0
        assert current(root) == "releases/v2-target"


def main() -> None:
    test_first_activation_and_atomic_replacement()
    test_rejects_incomplete_release_without_changing_current()
    test_rejects_unsafe_current_or_release_paths()
    test_rejects_symlinked_release_binary()
    test_idle_lock_refuses_activation_and_preserves_current_pair()
    print("install activation checks passed")


if __name__ == "__main__":
    main()
