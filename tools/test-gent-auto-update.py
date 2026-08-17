#!/usr/bin/env python3
"""Offline checks for the external opt-in automatic Gent runtime updater."""

from __future__ import annotations

import http.server
import fcntl
import json
import os
import platform
import socket
import subprocess
import sys
import tempfile
import threading
from functools import partial
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
UPDATER = ROOT / "tools" / "gent-auto-update.py"


class Server(http.server.ThreadingHTTPServer):
    allow_reuse_address = True


def installed(root: Path, version: str = "v1.2.3") -> Path:
    runtime = root / "install/lib/gent"
    release = runtime / f"releases/{version}-x86_64-unknown-linux-gnu"
    release.mkdir(parents=True)
    for name in ("gent", "gentd"):
        (release / name).write_text("binary", encoding="utf-8")
    (runtime / "current").symlink_to(Path("releases") / release.name)
    return runtime


def run(*arguments: str | Path, environment: dict[str, str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run([sys.executable, str(UPDATER), *map(str, arguments)], env=environment, text=True, capture_output=True)


def test_scheduler_files(runtime: Path, root: Path, environment: dict[str, str]) -> None:
    scheduler, data = root / "scheduler", root / "data"
    result = run("enable", "--runtime-root", runtime, "--data-dir", data, "--scheduler-dir", scheduler, "--interval-seconds", "600", environment=environment)
    assert result.returncode == 0, result.stderr
    if platform.system() == "Darwin":
        unit = scheduler / "ar.gent.auto-update.plist"
    else:
        unit = scheduler / "gent-auto-update.service"
        assert (scheduler / "gent-auto-update.timer").is_file()
    assert unit.is_file() and "gent-auto-update.py" in unit.read_text()
    result = run("status", "--runtime-root", runtime, "--data-dir", data, "--scheduler-dir", scheduler, environment=environment)
    assert json.loads(result.stdout)["enabled"]
    result = run("disable", "--runtime-root", runtime, "--data-dir", data, "--scheduler-dir", scheduler, environment=environment)
    assert result.returncode == 0 and not list(scheduler.glob("gent-auto-update.*"))


def test_tag_is_untrusted_but_installer_is_tag_bound(runtime: Path, root: Path, environment: dict[str, str]) -> None:
    files, record = root / "files", root / "record"
    (files / "v1.2.4").mkdir(parents=True)
    (files / "latest").write_text(json.dumps({"tag_name": "v1.2.4", "draft": False, "prerelease": False}), encoding="utf-8")
    (files / "v1.2.4/gent-install.sh").write_text("#!/usr/bin/env sh\nprintf '%s\\n' \"$@\" > \"$GENT_TEST_RECORD\"\n", encoding="utf-8")
    (files / "v1.2.4/gent-install.sh.sigstore.json").write_text("{}", encoding="utf-8")
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0)); port = probe.getsockname()[1]
    server = Server(("127.0.0.1", port), partial(http.server.SimpleHTTPRequestHandler, directory=str(files)))
    thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
    try:
        result = run("run", "--runtime-root", runtime, "--data-dir", root / "data", "--force", environment=environment | {"GENT_RELEASE_API_URL": f"http://127.0.0.1:{port}/latest", "GENT_RELEASE_DOWNLOAD_BASE_URL": f"http://127.0.0.1:{port}", "GENT_TEST_RECORD": str(record)})
    finally:
        server.shutdown(); thread.join(timeout=5)
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)["result"] == "updated"
    assert record.read_text(encoding="utf-8").splitlines() == ["--version", "v1.2.4", "--install-dir", str(root / "install"), "--idle-data-dir", str(root / "data"), "--force"]


def test_concurrent_run_is_refused(runtime: Path, root: Path, environment: dict[str, str]) -> None:
    with (runtime / ".auto-update.lock").open("a+b") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        result = run("run", "--runtime-root", runtime, "--data-dir", root / "data", environment=environment)
    assert result.returncode != 0 and "another automatic update" in result.stderr


def main() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary); fake = root / "fake"; fake.mkdir()
        cosign = fake / "cosign"; cosign.write_text("#!/usr/bin/env sh\nexit 0\n", encoding="utf-8"); cosign.chmod(0o755)
        environment = os.environ | {"PATH": f"{fake}:{os.environ['PATH']}"}
        runtime = installed(root)
        test_scheduler_files(runtime, root, environment)
        test_tag_is_untrusted_but_installer_is_tag_bound(runtime, root, environment)
        test_concurrent_run_is_refused(runtime, root, environment)
    print("automatic runtime update checks passed")


if __name__ == "__main__":
    main()
