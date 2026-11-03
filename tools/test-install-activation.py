#!/usr/bin/env python3

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import fcntl
import re
from pathlib import Path

from install_activation_installer_tests import (
    test_installer_allows_an_omitted_optional_digest_before_download,
    test_installer_rejects_malformed_versions_before_download,
)


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
    arguments.extend(("--idle-data-dir", str(data_dir or root / "fixture-data")))
    return subprocess.run(
        arguments,
        capture_output=True,
        text=True,
    )


def current(root: Path) -> str:
    selected = Path(os.readlink(root / "current")).name
    match = re.fullmatch(r"(v[0-9.]+)\+[0-9a-f]{16}\.[0-9a-f]{8}-(.+)", selected)
    assert match is not None, "new selections must use immutable qualified generations"
    return f"releases/{match.group(1)}-{match.group(2)}"


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
    command.extend(("--idle-data-dir", str(data or root / "fixture-data")))
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


def test_install_repairs_in_new_generation_and_preserves_tampered_old_files() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root, bin_dir = Path(temporary) / "gent", Path(temporary) / "bin"
        source = release(Path(temporary) / "source", "v1-target")
        assert install(root, "v1-target", source, bin_dir).returncode == 0
        old = (root / "current").resolve()
        installed = old / "gent"
        installed.write_text("tampered", encoding="utf-8")
        installed.chmod(0o755)
        result = install(root, "v1-target", source, bin_dir, force=True)
        assert result.returncode == 0, result.stderr
        assert (root / "current").resolve() != old
        assert installed.read_text() == "tampered"
        assert ((root / "current").resolve() / "gent").read_bytes() == (source / "gent").read_bytes()
        assert current(root) == "releases/v1-target"


def test_install_never_publishes_launchers_or_releases_before_idle_lease() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root, bin_dir, data = Path(temporary) / "gent", Path(temporary) / "bin", Path(temporary) / "data"
        source = release(Path(temporary) / "source", "v1-target")
        data.mkdir()
        with (data / "gentd.lock").open("a+b") as lock:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = install(root, "v1-target", source, bin_dir, data=data)
        assert result.returncode != 0
        assert not (root / "current").exists()
        assert not bin_dir.exists()
        assert not (root / "releases").exists()
        assert not list(root.glob(".gent-stage-*"))


def test_install_preserves_the_packaged_runtime_and_authority_trees() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root, bin_dir = Path(temporary) / "gent", Path(temporary) / "bin"
        source = release(Path(temporary) / "source", "v1-target")
        llama = source / "runtime" / "claurst" / "llama"
        llama.mkdir(parents=True)
        for path in (
            source / "runtime" / "node" / "bin" / "node",
            source / "runtime" / "node" / "bin" / "npm",
            source / "runtime" / "node" / "lib" / "node_modules" / "npm" / "bin" / "npm-cli.js",
            source / "runtime" / "claurst" / "claurst",
            llama / "llama-server",
            llama / "libllama.so",
            source / "authority" / "ordinary-authority.json",
            source / "authority" / "root-keys.json",
        ):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("runtime", encoding="utf-8")
            path.chmod(0o755)
        assert install(root, "v1-target", source, bin_dir).returncode == 0
        installed = (root / "current").resolve() / "runtime" / "claurst"
        assert (installed / "claurst").is_file()
        assert (installed / "llama" / "llama-server").is_file()
        assert (installed / "llama" / "libllama.so").is_file()
        assert (installed / "claurst").stat().st_mode & 0o111
        assert (installed / "llama" / "llama-server").stat().st_mode & 0o111
        authority = (root / "current").resolve() / "authority"
        assert (authority / "ordinary-authority.json").read_text(encoding="utf-8") == "runtime"
        assert (authority / "root-keys.json").is_file()


def test_stage_only_preserves_the_current_pair_for_later_health_checks() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root, bin_dir = Path(temporary) / "gent", Path(temporary) / "bin"
        first = release(Path(temporary) / "source", "v1-target")
        second = release(Path(temporary) / "source", "v2-target")
        assert install(root, "v1-target", first, bin_dir).returncode == 0
        result = subprocess.run(
            [sys.executable, str(ACTIVATOR), str(root), "v2-target", "--source-release", str(second), "--bin-dir", str(bin_dir), "--force", "--stage-only", "--idle-data-dir", str(root / "fixture-data")],
            capture_output=True, text=True,
        )
        assert result.returncode == 0 and "staged v2+" in result.stdout
        assert current(root) == "releases/v1-target"
        selected_stage = result.stdout.strip().split()[-1]
        assert (root / "releases" / selected_stage / "gentd").is_file()


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
                        "--idle-data-dir", str(root / "fixture-data"),
                    ],
                    stderr=subprocess.PIPE,
                    text=True,
                )
            )
        results = [process.wait(timeout=10) for process in commands]
        assert any(code == 0 for code in results)
        for process, code in zip(commands, results):
            if code:
                message = process.stderr.read()
                assert any(reason in message for reason in ("running", "in progress", "newer runtime"))
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
        assert ".install.lock" in result.stderr and "must be a real file" in result.stderr


def main() -> None:
    test_first_activation_and_atomic_replacement()
    test_rejects_incomplete_release_without_changing_current()
    test_rejects_unsafe_current_or_release_paths()
    test_rejects_symlinked_release_binary()
    test_idle_lock_refuses_activation_and_preserves_current_pair()
    test_install_repairs_in_new_generation_and_preserves_tampered_old_files()
    test_install_never_publishes_launchers_or_releases_before_idle_lease()
    test_install_preserves_the_packaged_runtime_and_authority_trees()
    test_stage_only_preserves_the_current_pair_for_later_health_checks()
    test_concurrent_installs_leave_no_stage_remnants()
    test_install_rejects_dangling_lock_symlink()
    test_installer_rejects_malformed_versions_before_download()
    test_installer_allows_an_omitted_optional_digest_before_download()
    print("install activation checks passed")


if __name__ == "__main__":
    main()
