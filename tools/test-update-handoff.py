#!/usr/bin/env python3
"""Exercise the user-invoked external update handoff without a live release."""

from __future__ import annotations

import hashlib
import http.server
import os
import platform
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
from functools import partial
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def target() -> str:
    system, machine = platform.system(), platform.machine().lower()
    values = {
        ("Darwin", "arm64"): "aarch64-apple-darwin",
        ("Darwin", "x86_64"): "x86_64-apple-darwin",
        ("Linux", "x86_64"): "x86_64-unknown-linux-gnu",
    }
    return values[(system, machine)]


def create_release(root: Path, version: str, runtime_target: str, include_supervisor: bool = True) -> str:
    source, output = root / "source", root / "output"
    source.mkdir(parents=True)
    output.mkdir()
    gent = source / "gent"
    gent.write_text("#!/usr/bin/env sh\nexit 0\n", encoding="utf-8")
    gent.chmod(0o755)
    gentd = source / "gentd"
    gentd.write_text(
        "#!/usr/bin/env sh\n"
        "data=\"$2\"\n"
        "mkdir -p \"$data\"\n"
        "touch \"$data/gentd.sock\"\n"
        "exec sleep 60\n",
        encoding="utf-8",
    )
    gentd.chmod(0o755)
    subprocess.run(
        [
            sys.executable,
            str(ROOT / "tools" / "package-release.py"),
            "--target-dir",
            str(source),
            "--out-dir",
            str(output),
            "--version",
            version,
            "--target",
            runtime_target,
            "--format",
            "tar.gz",
        ],
        check=True,
    )
    for path in output.iterdir():
        shutil.copy(path, root / path.name)
    archive = root / f"gent-{version}-{runtime_target}.tar.gz"
    for name in (
        f"{archive.name}.sigstore.json",
        f"{archive.name}.manifest.json.sigstore.json",
    ):
        (root / name).write_text("{}", encoding="utf-8")
    helpers = [
        (ROOT / "tools" / "install.sh", root / "gent-install.sh"),
        (ROOT / "tools" / "activate-install.py", root / "gent-activate-install.py"),
        (ROOT / "tools" / "gent-auto-update.py", root / "gent-auto-update.py"),
    ]
    if include_supervisor:
        helpers.append(
            (ROOT / "tools" / "supervise-runtime-activation.py", root / "gent-supervise-runtime-activation.py")
        )
    for source_path, destination in helpers:
        shutil.copy(source_path, destination)
        (root / f"{destination.name}.sigstore.json").write_text("{}", encoding="utf-8")
    return hashlib.sha256(archive.read_bytes()).hexdigest()


class Server(http.server.ThreadingHTTPServer):
    allow_reuse_address = True


def command(version: str, digest: str, install: Path, data: Path, env: dict[str, str]) -> list[str]:
    return [
        "cargo",
        "run",
        "--quiet",
        "-p",
        "gent-cli",
        "--bin",
        "gent",
        "--",
        "--data-dir",
        str(data),
        "update",
        "apply",
        "--version",
        version,
        "--expected-sha256",
        digest,
        "--install-dir",
        str(install),
        "--consent",
    ]


def held_lock(path: Path) -> subprocess.Popen[str]:
    script = """import fcntl, pathlib, sys
pathlib.Path(sys.argv[1]).parent.mkdir(parents=True, exist_ok=True)
with open(sys.argv[1], 'a+b') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    print('locked', flush=True)
    sys.stdin.read()
"""
    process = subprocess.Popen(
        [sys.executable, "-c", script, str(path)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
    )
    assert process.stdout and process.stdout.readline().strip() == "locked"
    return process


def main() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        work = Path(temporary)
        releases, install, data, fake = work / "releases", work / "install", work / "data", work / "bin"
        releases.mkdir()
        fake.mkdir()
        runtime_target = target()
        first = create_release(releases / "v1.2.3", "v1.2.3", runtime_target, include_supervisor=False)
        second = create_release(releases / "v1.2.4", "v1.2.4", runtime_target)
        third = create_release(releases / "v1.2.5", "v1.2.5", runtime_target)
        cosign = fake / "cosign"
        cosign.write_text("#!/usr/bin/env sh\nexit 0\n", encoding="utf-8")
        cosign.chmod(0o755)
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        handler = partial(http.server.SimpleHTTPRequestHandler, directory=str(releases))
        server = Server(("127.0.0.1", port), handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        env = os.environ | {
            "PATH": f"{fake}:{os.environ['PATH']}",
            "GENT_RUNTIME_ACTIVATION_TIMEOUT_SECONDS": "30",
        }
        try:
            env["GENT_RELEASE_BASE_URL"] = f"http://127.0.0.1:{port}/v1.2.3"
            subprocess.run(command("v1.2.3", first, install, data, env), cwd=ROOT, env=env, check=True)
            assert os.readlink(install / "lib" / "gent" / "current").endswith("v1.2.3-" + runtime_target)
            legacy = install / "lib" / "gent" / "releases" / f"v1.2.3-{runtime_target}"
            assert not (legacy / "supervise-runtime-activation.py").exists()
            assert (legacy / "gent-auto-update.py").is_file()
            env["GENT_RELEASE_BASE_URL"] = f"http://127.0.0.1:{port}/v1.2.4"
            subprocess.run(command("v1.2.4", second, install, data, env), cwd=ROOT, env=env, check=True)
            assert os.readlink(install / "lib" / "gent" / "current").endswith("v1.2.4-" + runtime_target)
            assert (install / "lib" / "gent" / "releases" / f"v1.2.4-{runtime_target}" / "supervise-runtime-activation.py").is_file()
            assert (install / "lib" / "gent" / "releases" / f"v1.2.4-{runtime_target}" / "gent-auto-update.py").is_file()
            env["GENT_RELEASE_BASE_URL"] = f"http://127.0.0.1:{port}/v1.2.5"
            env["GENT_RUNTIME_ACTIVATION_TIMEOUT_SECONDS"] = "1"
            lock = held_lock(data / "gentd.lock")
            try:
                result = subprocess.run(command("v1.2.5", third, install, data, env), cwd=ROOT, env=env)
                assert result.returncode != 0
            finally:
                assert lock.stdin
                lock.stdin.close()
                lock.wait(timeout=5)
            assert os.readlink(install / "lib" / "gent" / "current").endswith("v1.2.4-" + runtime_target)
        finally:
            server.shutdown()
            thread.join(timeout=5)
    print("external update handoff checks passed")


if __name__ == "__main__":
    main()
