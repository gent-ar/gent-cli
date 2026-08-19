#!/usr/bin/env python3
"""Build and Developer-ID sign the inert macOS Gent provider-helper bundle."""

from __future__ import annotations

import argparse
import os
import pathlib
import platform
import shutil
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
HELPER = ROOT / "platform" / "macos" / "provider-helper"
DEFAULT_OUTPUT = ROOT / "target" / "macos-provider-helper" / "GentProviderHelper.app"


def fail(message: str) -> None:
    raise SystemExit(f"build-macos-provider-helper: {message}")


def command(*args: str) -> None:
    subprocess.run(args, check=True)


def developer_id_identity(identity: str) -> bool:
    result = subprocess.run(
        ["security", "find-identity", "-v", "-p", "codesigning"],
        check=True,
        text=True,
        capture_output=True,
    )
    return any(
        identity in line and "Developer ID Application:" in line
        for line in result.stdout.splitlines()
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--identity",
        default=os.environ.get("GENT_MACOS_SIGNING_IDENTITY"),
        help="Developer ID Application identity or SHA-1 fingerprint",
    )
    parser.add_argument(
        "--output", type=pathlib.Path, default=DEFAULT_OUTPUT, help="output .app"
    )
    parser.add_argument(
        "--expected-team-id",
        default=os.environ.get("GENT_MACOS_SIGNING_TEAM_ID"),
        help="TeamIdentifier expected after signing",
    )
    parser.add_argument("--dry-run", action="store_true", help="print checks without writing")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if platform.system() != "Darwin":
        fail("macOS is required")
    if not args.identity:
        fail("pass --identity or set GENT_MACOS_SIGNING_IDENTITY")
    if not args.expected_team_id:
        fail("pass --expected-team-id or set GENT_MACOS_SIGNING_TEAM_ID")
    output = args.output.resolve()
    if output.suffix != ".app" or output == pathlib.Path("/"):
        fail("--output must name a .app bundle")
    if not developer_id_identity(args.identity):
        fail("identity is absent from Keychain or is not Developer ID Application")
    if args.dry_run:
        print(f"would build {output} with Developer ID identity {args.identity}")
        return
    binary = output / "Contents" / "MacOS" / "GentProviderHelper"
    if output.exists():
        shutil.rmtree(output)
    binary.parent.mkdir(parents=True)
    command("xcrun", "swiftc", "-O", str(HELPER / "Sources" / "main.swift"), "-o", str(binary))
    shutil.copy2(HELPER / "Info.plist", output / "Contents" / "Info.plist")
    command(
        "codesign",
        "--force",
        "--sign",
        args.identity,
        "--options",
        "runtime",
        "--timestamp",
        "--entitlements",
        str(HELPER / "GentProviderHelper.entitlements"),
        str(output),
    )
    command(
        sys.executable,
        str(ROOT / "tools" / "verify-macos-provider-helper.py"),
        str(output),
        "--expected-team-id",
        args.expected_team_id,
    )


if __name__ == "__main__":
    main()
