#!/usr/bin/env python3
"""Static contract checks for macOS provider-helper release tooling."""

from __future__ import annotations

import pathlib
import plistlib
import shutil
import subprocess
import sys
import tempfile
import json


ROOT = pathlib.Path(__file__).resolve().parents[1]
HELPER = ROOT / "platform" / "macos" / "provider-helper"


def expect(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def protocol_request() -> dict[str, object]:
    return {
        "protocolVersion": 1,
        "requestId": "test-request",
        "operation": "prepare",
        "helper": {"bundleId": "io.gent.provider-helper", "version": "0.1.0"},
        "provider": {
            "name": "codex", "canonicalPath": "/private/codex", "fileIdentity": "1:2",
            "digestSha256": "a" * 64, "version": "1", "compatibilityEntry": "codex-1",
        },
        "profile": {
            "profileDigestSha256": "b" * 64,
            "network": {"mode": "disabled", "egressPolicyDigestSha256": None},
            "limits": {"maxProcesses": 1, "maxMemoryBytes": 1, "maxCpuTimeMs": 1},
        },
    }


def run_protocol(binary: pathlib.Path, request: dict[str, object]) -> tuple[int, dict[str, object]]:
    result = subprocess.run(
        [str(binary), "--protocol"], input=json.dumps(request), text=True, capture_output=True
    )
    return result.returncode, json.loads(result.stdout)


def check_protocol() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = pathlib.Path(directory)
        app = root / "GentProviderHelper.app"
        binary = app / "Contents" / "MacOS" / "GentProviderHelper"
        binary.parent.mkdir(parents=True)
        subprocess.run(
            ["xcrun", "swiftc", "-O", str(HELPER / "Sources" / "main.swift"), "-o", str(binary)],
            check=True,
        )
        shutil.copy2(HELPER / "Info.plist", app / "Contents" / "Info.plist")
        code, response = run_protocol(binary, protocol_request())
        expect(code == 0, "valid protocol request failed")
        expect(response["result"] == {"state": "denied", "reason": "workspaceBookmarkRequired"}, "bookmark denial changed")
        invalid = protocol_request()
        invalid["provider"]["name"] = "shell"  # type: ignore[index]
        code, response = run_protocol(binary, invalid)
        expect(code == 65, "invalid provider lock was accepted")
        expect(response["result"] == {"state": "invalidRequest", "reason": "invalidProviderLock"}, "bad lock error changed")


def main() -> None:
    with (HELPER / "Info.plist").open("rb") as stream:
        info = plistlib.load(stream)
    with (HELPER / "GentProviderHelper.entitlements").open("rb") as stream:
        entitlements = plistlib.load(stream)
    expect(info["CFBundleIdentifier"] == "io.gent.provider-helper", "bad helper id")
    expect(info["CFBundleExecutable"] == "GentProviderHelper", "bad helper executable")
    expect(entitlements == {"com.apple.security.app-sandbox": True}, "broad entitlement")
    source = (HELPER / "Sources" / "main.swift").read_text()
    expect("Process(" not in source and "NSTask" not in source, "helper spawns a process")
    expect("launchPath" not in source and "executableURL" not in source, "helper accepts process launch")
    expect("workspaceBookmarkRequired" in source, "helper lacks workspace denial")
    with tempfile.TemporaryDirectory() as directory:
        result = subprocess.run(
            [
                sys.executable,
                str(ROOT / "tools" / "build-macos-provider-helper.py"),
                "--output",
                str(pathlib.Path(directory) / "not-an-app"),
            ],
            text=True,
            capture_output=True,
        )
    expect(result.returncode != 0, "builder accepted a non-app output")
    check_protocol()
    print("macOS provider-helper tooling contracts passed")


if __name__ == "__main__":
    main()
