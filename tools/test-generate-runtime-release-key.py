#!/usr/bin/env python3
"""Check that generated runtime release keys are new, private, and signer-compatible."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
GENERATOR = ROOT / "tools" / "generate-runtime-release-key.py"
SIGNER = ROOT / "tools" / "sign-runtime-release.py"


def module():
    spec = importlib.util.spec_from_file_location("runtime_release_signer", SIGNER)
    assert spec and spec.loader
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def main() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root, key = Path(temporary), Path(temporary) / "release.pem"
        result = subprocess.run(
            [sys.executable, str(GENERATOR), "--key-id", "release-1", "--private-key-out", str(key)],
            check=True,
            capture_output=True,
            text=True,
        )
        lines = dict(line.split("=", 1) for line in result.stdout.splitlines())
        assert key.stat().st_mode & 0o777 == 0o600
        assert lines["GENT_RUNTIME_RELEASE_KEY_ID"] == "release-1"
        assert lines["GENT_RUNTIME_RELEASE_PUBLIC_KEY"] == module().public_key(module().load_seed(key)).hex()
        assert subprocess.run(
            [sys.executable, str(GENERATOR), "--key-id", "release-1", "--private-key-out", str(key)],
            capture_output=True,
            text=True,
        ).returncode != 0
        assert not (root / "unwanted.pem").exists()
    print("runtime release key generation checks passed")


if __name__ == "__main__":
    main()
