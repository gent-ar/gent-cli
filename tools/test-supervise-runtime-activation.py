#!/usr/bin/env python3
"""Offline activation-supervisor checks using a local fake Gent pair."""

from __future__ import annotations

import os
import signal
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
ACTIVATOR = ROOT / "tools/activate-install.py"
SUPERVISOR = ROOT / "tools/supervise-runtime-activation.py"


def source(root: Path, name: str) -> Path:
    directory = root / name
    directory.mkdir(parents=True)
    (directory / "gent").write_text("#!/bin/sh\nexit 0\n")
    (directory / "gentd").write_text(
        "#!/bin/sh\ndata=\"$2\"\nmkdir -p \"$data\"\nprintf '%s' \"$$\" > \"$data/gentd.pid\"\ntouch \"$data/gentd.sock\"\nexec sleep 60\n"
    )
    for binary in (directory / "gent", directory / "gentd"):
        binary.chmod(0o755)
    return directory


def activate(root: Path, name: str, source_dir: Path, bin_dir: Path) -> None:
    subprocess.run([sys.executable, str(ACTIVATOR), str(root), name, "--source-release", str(source_dir), "--source-supervisor", str(SUPERVISOR), "--bin-dir", str(bin_dir)], check=True)


def main() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        work = Path(temporary)
        runtime, bin_dir, data = work / ".local/lib/gent", work / ".local/bin", work / "data"
        first, second = source(work / "source", "v1"), source(work / "source", "v2")
        activate(runtime, "v1", first, bin_dir)
        staged = runtime / "releases/v1/supervise-runtime-activation.py"
        material = work / "update-material"
        material.mkdir()
        (material / "runtime-release-cache.json").write_text("{}")
        (material / "runtime-release-trust.json").write_text("{}")
        subprocess.run([sys.executable, str(staged), "--runtime-root", str(runtime), "--release-name", "v2", "--source-release", str(second), "--bin-dir", str(bin_dir), "--data-dir", str(data)], check=True)
        assert os.readlink(runtime / "current") == "releases/v2"
        assert (runtime / "releases/v2/activate-install.py").is_file()
        assert not list((runtime / "releases").glob(".gent-stage-*"))
        subprocess.run([sys.executable, str(staged), "--runtime-root", str(runtime), "--release-name", "v3", "--source-release", str(second), "--source-update-material", str(material), "--bin-dir", str(bin_dir), "--data-dir", str(data)], check=True)
        assert (runtime / "releases/v3/runtime-release-cache.json").read_text() == "{}"
        assert (runtime / "releases/v3/runtime-release-trust.json").read_text() == "{}"
        cache, trust = work / "release.json", work / "trust.json"
        cache.write_text("{}")
        trust.write_text("{}")
        subprocess.run([sys.executable, str(staged), "--runtime-root", str(runtime), "--release-name", "v2", "--source-release", str(second), "--bin-dir", str(bin_dir), "--data-dir", str(data), "--recover-attempt-id", "attempt-1", "--runtime-release-cache", str(cache), "--runtime-release-trust", str(trust)], check=True)
        successor = int((data / "gentd.pid").read_text())
        os.kill(successor, signal.SIGTERM)
    print("runtime activation supervisor checks passed")


if __name__ == "__main__":
    main()
