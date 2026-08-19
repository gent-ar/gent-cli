#!/usr/bin/env python3
"""Fail closed when a Gent macOS provider-helper bundle is not release signed."""

from __future__ import annotations

import argparse
import pathlib
import plistlib
import platform
import subprocess


EXPECTED_BUNDLE_ID = "io.gent.provider-helper"
EXPECTED_EXECUTABLE = "GentProviderHelper"


def fail(message: str) -> None:
    raise SystemExit(f"verify-macos-provider-helper: {message}")


def displayed_signature(bundle: pathlib.Path) -> str:
    result = subprocess.run(
        ["codesign", "-dvv", str(bundle)], text=True, capture_output=True, check=True
    )
    return result.stderr


def signed_entitlements(bundle: pathlib.Path) -> dict[str, object]:
    result = subprocess.run(
        ["codesign", "-d", "--entitlements", ":-", str(bundle)],
        text=True,
        capture_output=True,
        check=True,
    )
    return plistlib.loads(result.stdout.encode())


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=pathlib.Path)
    parser.add_argument("--expected-team-id", required=True)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if platform.system() != "Darwin":
        fail("macOS is required")
    bundle = args.bundle.resolve()
    executable = bundle / "Contents" / "MacOS" / EXPECTED_EXECUTABLE
    info_path = bundle / "Contents" / "Info.plist"
    if bundle.suffix != ".app" or not executable.is_file() or not info_path.is_file():
        fail("expected a complete .app bundle")
    with info_path.open("rb") as stream:
        info = plistlib.load(stream)
    if info.get("CFBundleIdentifier") != EXPECTED_BUNDLE_ID:
        fail("unexpected bundle identifier")
    if info.get("CFBundleExecutable") != EXPECTED_EXECUTABLE:
        fail("unexpected executable name")
    subprocess.run(
        ["codesign", "--verify", "--deep", "--strict", "--verbose=2", str(bundle)],
        check=True,
    )
    signature = displayed_signature(bundle)
    required = (
        "Authority=Developer ID Application:",
        f"TeamIdentifier={args.expected_team_id}",
        "flags=0x10000(runtime)",
    )
    if any(item not in signature for item in required):
        fail("missing Developer ID authority, expected team, or hardened runtime")
    entitlements = signed_entitlements(bundle)
    if entitlements != {"com.apple.security.app-sandbox": True}:
        fail("signed entitlement set is not the helper's least-privilege sandbox")
    print(f"verified {bundle}")


if __name__ == "__main__":
    main()
