#!/usr/bin/env python3
"""Prove the signed installer persists only Gentd-verified update material."""

from __future__ import annotations

import http.server
import json
import os
import platform
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from functools import partial
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def runtime_target() -> str:
    targets = {
        ("Darwin", "arm64"): "aarch64-apple-darwin",
        ("Darwin", "x86_64"): "x86_64-apple-darwin",
        ("Linux", "x86_64"): "x86_64-unknown-linux-gnu",
    }
    return targets[(platform.system(), platform.machine().lower())]


def release_version() -> str:
    metadata = subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        text=True,
    )
    packages = {package["name"]: package["version"] for package in json.loads(metadata)["packages"]}
    version = packages.get("gentd")
    if version is None or version != packages.get("gent-cli"):
        raise ValueError("gent and gentd must share a workspace release version")
    return f"v{version}"


def command(*values: str | Path) -> None:
    subprocess.run([str(value) for value in values], cwd=ROOT, check=True)


def public_key(output: str) -> str:
    for line in output.splitlines():
        if line.startswith("GENT_RUNTIME_RELEASE_PUBLIC_KEY="):
            return line.partition("=")[2]
    raise AssertionError("key generator did not report a public key")


def release(directory: Path, target: str, version: str) -> None:
    command("cargo", "build", "--quiet", "-p", "gent-cli", "-p", "gentd", "--bins")
    command(
        sys.executable, ROOT / "tools/package-release.py", "--target-dir", ROOT / "target/debug",
        "--out-dir", directory, "--version", version, "--target", target, "--format", "tar.gz",
    )
    key = directory / "release-private.pem"
    generated = subprocess.run(
        [sys.executable, str(ROOT / "tools/generate-runtime-release-key.py"), "--key-id", "test-release", "--private-key-out", str(key)],
        cwd=ROOT, check=True, text=True, capture_output=True,
    )
    (directory / "gent-runtime-release-trust.json").write_text(
        json.dumps({"schemaVersion": 1, "keys": [{"keyId": "test-release", "publicKeyHex": public_key(generated.stdout)}]}),
        encoding="utf-8",
    )
    archive = directory / f"gent-{version}-{target}.tar.gz"
    metadata = directory / f"gent-{version}-{target}.runtime-release.json"
    command(
        sys.executable, ROOT / "tools/sign-runtime-release.py", "--archive-manifest", f"{archive}.manifest.json",
        "--version", version, "--target", target, "--key-id", "test-release", "--private-key", key,
        "--expires-at", str(int(time.time()) + 3600), "--out", metadata,
    )
    key.unlink()
    for source, name in (
        (ROOT / "tools/activate-install.py", "gent-activate-install.py"),
        (ROOT / "tools/gent-auto-update.py", "gent-auto-update.py"),
    ):
        shutil.copy(source, directory / name)
    for path in directory.iterdir():
        if path.name.endswith((".tar.gz", ".manifest.json", ".runtime-release.json")) or path.name in {
            "gent-activate-install.py", "gent-auto-update.py", "gent-runtime-release-trust.json",
        }:
            Path(f"{path}.sigstore.json").write_text("{}", encoding="utf-8")


class Server(http.server.ThreadingHTTPServer):
    allow_reuse_address = True


def main() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        releases, install, fake = root / "releases", root / "install", root / "fake"
        releases.mkdir()
        fake.mkdir()
        target, version = runtime_target(), release_version()
        release(releases, target, version)
        cosign = fake / "cosign"
        cosign.write_text("#!/usr/bin/env sh\nexit 0\n", encoding="utf-8")
        cosign.chmod(0o755)
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        server = Server(("127.0.0.1", port), partial(http.server.SimpleHTTPRequestHandler, directory=str(releases)))
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        environment = os.environ | {"PATH": f"{fake}:{os.environ['PATH']}", "GENT_RELEASE_BASE_URL": f"http://127.0.0.1:{port}"}
        try:
            command_result = subprocess.run(
                ["sh", str(ROOT / "tools/install.sh"), "--version", version, "--install-dir", str(install)],
                cwd=ROOT, env=environment, capture_output=True, text=True,
            )
            assert command_result.returncode == 0, command_result.stderr
        finally:
            server.shutdown()
            thread.join(timeout=5)
        staged = install / "lib/gent/releases" / f"{version}-{target}"
        assert (staged / "gent-auto-update.py").is_file()
        cache = json.loads((staged / "runtime-release-cache.json").read_text(encoding="utf-8"))
        assert cache["release"]["keyId"] == "test-release"
        assert (staged / "runtime-release-trust.json").stat().st_mode & 0o777 == 0o600
        assert (install / "lib/gent/gent-auto-update.py").is_file()
    print("installer runtime bootstrap checks passed")


if __name__ == "__main__":
    main()
