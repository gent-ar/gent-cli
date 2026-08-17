#!/usr/bin/env python3
"""Offline tests for immutable staged runtime update trust and cache material."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
ACTIVATOR = ROOT / "tools" / "activate-install.py"


def release(root: Path) -> Path:
    root.mkdir(parents=True)
    for name in ("gent", "gentd"):
        path = root / name
        path.write_text("#!/usr/bin/env sh\nexit 0\n", encoding="utf-8")
        path.chmod(0o755)
    return root


def material(root: Path, marker: str) -> Path:
    root.mkdir(parents=True)
    (root / "runtime-release-trust.json").write_text(
        json.dumps({"schemaVersion": 1, "keys": [{"keyId": marker, "publicKeyHex": "a" * 64}]}),
        encoding="utf-8",
    )
    (root / "runtime-release-cache.json").write_text(
        json.dumps({"verifiedAtUnixSeconds": 1, "release": {"keyId": marker}}),
        encoding="utf-8",
    )
    return root


def install(runtime: Path, source: Path, update: Path, bin_dir: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            sys.executable, str(ACTIVATOR), str(runtime), "v1-target", "--source-release",
            str(source), "--source-update-material", str(update), "--bin-dir", str(bin_dir), "--force",
        ],
        capture_output=True,
        text=True,
    )


def main() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        runtime, bin_dir = root / "runtime", root / "bin"
        source = release(root / "source")
        original = material(root / "material", "release-1")
        assert install(runtime, source, original, bin_dir).returncode == 0
        staged = runtime / "releases" / "v1-target"
        assert json.loads((staged / "runtime-release-trust.json").read_text())["keys"][0]["keyId"] == "release-1"
        assert (staged / "runtime-release-trust.json").stat().st_mode & 0o777 == 0o600
        changed = material(root / "changed", "release-2")
        result = install(runtime, source, changed, bin_dir)
        assert result.returncode != 0
        assert "verified update material" in result.stderr
        assert json.loads((staged / "runtime-release-cache.json").read_text())["release"]["keyId"] == "release-1"
    print("install update material checks passed")


if __name__ == "__main__":
    main()
