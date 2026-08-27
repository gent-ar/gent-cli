from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
INSTALLER = ROOT / "tools" / "install.sh"


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


def test_installer_allows_an_omitted_optional_digest_before_download() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        fake, marker = Path(temporary) / "fake", Path(temporary) / "downloaded"
        fake.mkdir()
        curl = fake / "curl"
        curl.write_text(f"#!/usr/bin/env sh\ntouch {marker}\nexit 99\n", encoding="utf-8")
        curl.chmod(0o755)
        cosign = fake / "cosign"
        cosign.write_text("#!/usr/bin/env sh\nexit 0\n", encoding="utf-8")
        cosign.chmod(0o755)
        result = subprocess.run(
            ["sh", str(INSTALLER), "--version", "v1.2.3"],
            env=os.environ | {"PATH": f"{fake}:{os.environ['PATH']}"},
            capture_output=True,
            text=True,
        )
        assert result.returncode != 0
        assert marker.exists()
        assert "expected digest" not in result.stderr
