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
INSTALLER = ROOT / "tools" / "install.sh"


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


def install(
    root: Path, name: str, source: Path, bin_dir: Path, *, force: bool = False, data: Path | None = None
) -> subprocess.CompletedProcess[str]:
    command = [
        sys.executable,
        str(ACTIVATOR),
        str(root),
        name,
        "--source-release",
        str(source),
        "--bin-dir",
        str(bin_dir),
    ]
    if force:
        command.append("--force")
    if data is not None:
        command.extend(("--idle-data-dir", str(data)))
    return subprocess.run(command, capture_output=True, text=True)


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


def test_install_rejects_tampered_existing_release_and_preserves_pointer() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root, bin_dir = Path(temporary) / "gent", Path(temporary) / "bin"
        source = release(Path(temporary) / "source", "v1-target")
        assert install(root, "v1-target", source, bin_dir).returncode == 0
        installed = root / "releases" / "v1-target" / "gent"
        installed.write_text("tampered", encoding="utf-8")
        installed.chmod(0o755)
        result = install(root, "v1-target", source, bin_dir, force=True)
        assert result.returncode != 0
        assert "differs from the verified release" in result.stderr
        assert current(root) == "releases/v1-target"


def test_install_publishes_launchers_before_idle_pointer_activation() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root, bin_dir, data = Path(temporary) / "gent", Path(temporary) / "bin", Path(temporary) / "data"
        source = release(Path(temporary) / "source", "v1-target")
        data.mkdir()
        with (data / "gentd.lock").open("a+b") as lock:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = install(root, "v1-target", source, bin_dir, data=data)
        assert result.returncode != 0
        assert not (root / "current").exists()
        assert all((bin_dir / name).is_file() for name in ("gent", "gentd"))
        assert not list((root / "releases").glob(".gent-stage-*"))


def test_stage_only_preserves_the_current_pair_for_later_health_checks() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root, bin_dir = Path(temporary) / "gent", Path(temporary) / "bin"
        first = release(Path(temporary) / "source", "v1-target")
        second = release(Path(temporary) / "source", "v2-target")
        assert install(root, "v1-target", first, bin_dir).returncode == 0
        result = subprocess.run(
            [sys.executable, str(ACTIVATOR), str(root), "v2-target", "--source-release", str(second), "--bin-dir", str(bin_dir), "--force", "--stage-only"],
            capture_output=True, text=True,
        )
        assert result.returncode == 0 and "staged v2-target" in result.stdout
        assert current(root) == "releases/v1-target"
        assert (root / "releases" / "v2-target" / "gentd").is_file()


def test_concurrent_installs_leave_no_stage_remnants() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root, bin_dir = Path(temporary) / "gent", Path(temporary) / "bin"
        source = Path(temporary) / "source"
        first, second = release(source, "v1-target"), release(source, "v2-target")
        commands = []
        for name, directory in (("v1-target", first), ("v2-target", second)):
            commands.append(
                subprocess.Popen(
                    [
                        sys.executable,
                        str(ACTIVATOR),
                        str(root),
                        name,
                        "--source-release",
                        str(directory),
                        "--bin-dir",
                        str(bin_dir),
                        "--force",
                    ],
                    stderr=subprocess.PIPE,
                    text=True,
                )
            )
        assert all(process.wait(timeout=10) == 0 for process in commands)
        assert current(root) in {"releases/v1-target", "releases/v2-target"}
        assert not list((root / "releases").glob(".gent-stage-*"))


def test_install_rejects_dangling_lock_symlink() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root, bin_dir = Path(temporary) / "gent", Path(temporary) / "bin"
        root.mkdir()
        (root / ".install.lock").symlink_to("missing-lock")
        source = release(Path(temporary) / "source", "v1-target")
        result = install(root, "v1-target", source, bin_dir)
        assert result.returncode != 0
        assert "install lock must be a real file" in result.stderr


def test_installer_rejects_malformed_versions_before_download() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        fake = Path(temporary) / "fake"
        fake.mkdir()
        marker = Path(temporary) / "downloaded"
        for name in ("curl", "cosign"):
            binary = fake / name
            binary.write_text(f"#!/usr/bin/env sh\ntouch {marker}\nexit 99\n", encoding="utf-8")
            binary.chmod(0o755)
        for version in ("v1.2.3.*", "v1.2.3/escape", "v1.2.3 extra"):
            result = subprocess.run(
                ["sh", str(INSTALLER), "--version", version],
                env=os.environ | {"PATH": f"{fake}:{os.environ['PATH']}"},
                capture_output=True,
                text=True,
            )
            assert result.returncode != 0
            assert "invalid release version" in result.stderr
        assert not marker.exists()


def main() -> None:
    test_first_activation_and_atomic_replacement()
    test_rejects_incomplete_release_without_changing_current()
    test_rejects_unsafe_current_or_release_paths()
    test_rejects_symlinked_release_binary()
    test_idle_lock_refuses_activation_and_preserves_current_pair()
    test_install_rejects_tampered_existing_release_and_preserves_pointer()
    test_install_publishes_launchers_before_idle_pointer_activation()
    test_stage_only_preserves_the_current_pair_for_later_health_checks()
    test_concurrent_installs_leave_no_stage_remnants()
    test_install_rejects_dangling_lock_symlink()
    test_installer_rejects_malformed_versions_before_download()
    print("install activation checks passed")


if __name__ == "__main__":
    main()
