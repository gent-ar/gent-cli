#!/usr/bin/env python3
"""Run or install an opt-in external Gent paired-runtime update scheduler."""

from __future__ import annotations

import argparse
from contextlib import contextmanager
from html import escape
import json
import os
import platform
import re
import stat
import subprocess
import sys
import tempfile
import time
import urllib.request
import urllib.error
from pathlib import Path

try:
    import fcntl
except ImportError:  # Windows ships the immutable pair but has no scheduler yet.
    fcntl = None


REPOSITORY = "gent-ar/gent-cli"
TAG = re.compile(r"v([0-9]+)\.([0-9]+)\.([0-9]+)$")
RELEASE = re.compile(r"(v[0-9]+\.[0-9]+\.[0-9]+)-.+$")
MIN_INTERVAL, MAX_INTERVAL = 300, 7 * 24 * 60 * 60


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("run", "enable", "disable", "status"))
    parser.add_argument("--runtime-root", type=Path, required=True)
    parser.add_argument("--data-dir", type=Path, required=True)
    parser.add_argument("--interval-seconds", type=int, default=6 * 60 * 60)
    parser.add_argument("--timeout-seconds", type=int, default=30)
    parser.add_argument("--scheduler-dir", type=Path)
    parser.add_argument("--force", action="store_true")
    return parser.parse_args()


def fail(message: str) -> None:
    raise ValueError(f"Gent automatic update refused: {message}")


def require_install(root: Path) -> tuple[Path, tuple[int, int, int]]:
    current = root / "current"
    if not root.is_dir() or not current.is_symlink():
        fail("an installed Gent runtime pair is required")
    target = Path(os.readlink(current))
    if target.parent != Path("releases"):
        fail("current runtime pointer is not managed")
    release = RELEASE.fullmatch(target.name)
    match = TAG.fullmatch(release.group(1)) if release else None
    binaries = (root / target / "gent", root / target / "gentd")
    if match is None or any(path.is_symlink() or not stat.S_ISREG(path.stat().st_mode) for path in binaries):
        fail("current runtime pair is incomplete")
    return root.parent.parent, tuple(map(int, match.groups()))


def runtime_file(root: Path) -> Path:
    return root / "auto-update-state.json"


def read_state(root: Path) -> dict[str, object]:
    path = runtime_file(root)
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return {"schemaVersion": 1, "failureCount": 0, "nextEligibleAt": 0}
    except (OSError, json.JSONDecodeError) as error:
        fail(f"invalid automatic-update state: {error}")
    if not isinstance(value, dict) or value.get("schemaVersion") != 1:
        fail("automatic-update state schema is unsupported")
    return value


def save_state(root: Path, value: dict[str, object]) -> None:
    path = runtime_file(root)
    temporary = path.with_name(f".{path.name}.{os.getpid()}")
    with temporary.open("w", encoding="utf-8") as output:
        output.write(json.dumps(value, separators=(",", ":")) + "\n")
        output.flush(); os.fsync(output.fileno())
    temporary.chmod(0o600)
    os.replace(temporary, path)
    descriptor = os.open(root, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


@contextmanager
def update_lock(root: Path):
    if fcntl is None:
        fail("automatic updates are currently supported only on macOS and Linux")
    descriptor = os.open(root / ".auto-update.lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "a+b") as lock:
        try:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            fail("another automatic update is already running")
        try:
            yield
        finally:
            fcntl.flock(lock.fileno(), fcntl.LOCK_UN)


def release_tag() -> str:
    endpoint = os.environ.get("GENT_RELEASE_API_URL", f"https://api.github.com/repos/{REPOSITORY}/releases/latest")
    with urllib.request.urlopen(endpoint, timeout=10) as response:  # nosec B310: tag is untrusted discovery only
        release = json.load(response)
    if not isinstance(release, dict):
        fail("release discovery returned an invalid document")
    tag = release.get("tag_name")
    if release.get("draft") or release.get("prerelease") or not isinstance(tag, str) or TAG.fullmatch(tag) is None:
        fail("release discovery did not return a stable semantic version tag")
    return tag


def invoke(command: list[str], timeout: int) -> None:
    subprocess.run(command, check=True, timeout=timeout, capture_output=True, text=True)


def update(root: Path, data: Path, timeout: int, force: bool) -> dict[str, object]:
    install_root, installed = require_install(root)
    state, now = read_state(root), int(time.time())
    eligible = state.get("nextEligibleAt", 0)
    if not force and isinstance(eligible, int) and eligible > now:
        return state | {"result": "backoff"}
    try:
        tag = release_tag()
        selected = tuple(map(int, TAG.fullmatch(tag).groups()))
        if selected <= installed:
            result = state | {"failureCount": 0, "nextEligibleAt": 0, "lastSeenTag": tag, "result": "current"}
        else:
            with tempfile.TemporaryDirectory(prefix="gent-auto-update-") as directory:
                script = Path(directory) / "gent-install.sh"
                base = os.environ.get("GENT_RELEASE_DOWNLOAD_BASE_URL", f"https://github.com/{REPOSITORY}/releases/download")
                url = f"{base.rstrip('/')}/{tag}/gent-install.sh"
                invoke(["curl", "--fail", "--location", "--silent", "--show-error", "--output", str(script), url], timeout)
                invoke(["curl", "--fail", "--location", "--silent", "--show-error", "--output", str(script) + ".sigstore.json", url + ".sigstore.json"], timeout)
                identity = f"^https://github.com/{REPOSITORY}/.github/workflows/release.yml@refs/tags/{tag}$"
                invoke(["cosign", "verify-blob", str(script), "--bundle", str(script) + ".sigstore.json", "--certificate-identity-regexp", identity, "--certificate-oidc-issuer", "https://token.actions.githubusercontent.com"], timeout)
                environment = os.environ | {"GENT_RUNTIME_ACTIVATION_TIMEOUT_SECONDS": str(timeout)}
                subprocess.run(["sh", str(script), "--version", tag, "--install-dir", str(install_root), "--idle-data-dir", str(data), "--force"], check=True, timeout=timeout * 4, env=environment, capture_output=True, text=True)
            result = {"schemaVersion": 1, "failureCount": 0, "nextEligibleAt": 0, "lastSeenTag": tag, "lastUpdatedAt": now, "result": "updated"}
    except (OSError, ValueError, subprocess.SubprocessError, TimeoutError, urllib.error.URLError) as error:
        failures = int(state.get("failureCount", 0)) + 1
        result = state | {"failureCount": min(failures, 6), "nextEligibleAt": now + min(6 * 60 * 60, 60 * (2 ** min(failures, 6))), "lastError": str(error), "result": "failed"}
    save_state(root, result)
    return result


def scheduler_root(values: argparse.Namespace) -> Path:
    if values.scheduler_dir is not None:
        return values.scheduler_dir
    if platform.system() == "Darwin":
        return Path.home() / "Library" / "LaunchAgents"
    return Path.home() / ".config" / "systemd" / "user"


def schedule_paths(values: argparse.Namespace) -> tuple[Path, Path | None]:
    root = scheduler_root(values)
    if platform.system() == "Darwin":
        return root / "ar.gent.auto-update.plist", None
    return root / "gent-auto-update.service", root / "gent-auto-update.timer"


def command_line(values: argparse.Namespace) -> list[str]:
    return [sys.executable, str(values.runtime_root / "gent-auto-update.py"), "run", "--runtime-root", str(values.runtime_root), "--data-dir", str(values.data_dir), "--timeout-seconds", str(values.timeout_seconds)]


def systemd_argument(value: str) -> str:
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"').replace("$", "$$") + '"'


def enable(values: argparse.Namespace) -> None:
    require_install(values.runtime_root)
    if not MIN_INTERVAL <= values.interval_seconds <= MAX_INTERVAL:
        fail(f"interval must be {MIN_INTERVAL}..{MAX_INTERVAL} seconds")
    first, second = schedule_paths(values)
    first.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    command = command_line(values)
    if second is None:
        arguments = "".join(f"<string>{escape(part)}</string>" for part in command)
        first.write_text("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>ar.gent.auto-update</string><key>ProgramArguments</key><array>" + arguments + f"</array><key>StartInterval</key><integer>{values.interval_seconds}</integer></dict></plist>\n", encoding="utf-8")
        if values.scheduler_dir is None:
            subprocess.run(["launchctl", "bootstrap", f"gui/{os.getuid()}", str(first)], check=True, timeout=10)
    else:
        first.write_text("[Unit]\nDescription=Gent automatic paired runtime update\n[Service]\nType=oneshot\nExecStart=" + " ".join(map(systemd_argument, command)) + "\n", encoding="utf-8")
        second.write_text("[Unit]\nDescription=Run Gent automatic update\n[Timer]\nOnBootSec=5m\nOnUnitActiveSec=" + str(values.interval_seconds) + "\n[Install]\nWantedBy=timers.target\n", encoding="utf-8")
        if values.scheduler_dir is None:
            subprocess.run(["systemctl", "--user", "daemon-reload"], check=True, timeout=10)
            subprocess.run(["systemctl", "--user", "enable", "--now", second.name], check=True, timeout=10)


def disable(values: argparse.Namespace) -> None:
    first, second = schedule_paths(values)
    if values.scheduler_dir is None:
        if second is None and first.exists():
            subprocess.run(["launchctl", "bootout", f"gui/{os.getuid()}", str(first)], timeout=10)
        elif second is not None:
            subprocess.run(["systemctl", "--user", "disable", "--now", second.name], timeout=10)
    for path in (first, second):
        if path is not None:
            path.unlink(missing_ok=True)


def main() -> None:
    values = arguments()
    if fcntl is None:
        fail("automatic updates are currently supported only on macOS and Linux")
    if not 1 <= values.timeout_seconds <= 120:
        fail("timeout must be 1..120 seconds")
    if values.action == "run":
        with update_lock(values.runtime_root):
            print(json.dumps(update(values.runtime_root, values.data_dir, values.timeout_seconds, values.force), sort_keys=True))
    elif values.action == "enable":
        enable(values)
    elif values.action == "disable":
        disable(values)
    else:
        first, second = schedule_paths(values)
        print(json.dumps(read_state(values.runtime_root) | {"enabled": first.exists() and (second is None or second.exists())}, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError, TimeoutError) as error:
        raise SystemExit(str(error)) from error
