#!/usr/bin/env python3
import argparse
import json
import pathlib
import sys

from provider_pins import PINS, load_pins, platform_tarball_digests, tarball_mismatches


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Digest the executed native binary inside a pinned provider platform tarball")
    parser.add_argument("provider", choices=("claude", "codex"))
    parser.add_argument("target")
    parser.add_argument("tarball", type=pathlib.Path)
    parser.add_argument("--pins", type=pathlib.Path, default=PINS)
    parser.add_argument("--package-name", help="platform package name for a target not yet pinned")
    parser.add_argument("--executable", help="pins.json-style executable path for a target not yet pinned")
    parser.add_argument("--check", action="store_true", help="fail unless the tarball matches pins.json")
    return parser.parse_args()


def main() -> int:
    args = arguments()
    pins = load_pins(args.pins)
    detail = pins["providers"][args.provider]["targets"].get(args.target, {})
    package_name = args.package_name or detail["package"]["name"]
    executable = args.executable or detail["executable"]
    print(json.dumps(platform_tarball_digests(args.tarball, package_name, executable), indent=2))
    if not args.check:
        return 0
    errors = tarball_mismatches(args.tarball, args.provider, args.target, pins)
    for error in errors:
        print(error, file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        raise SystemExit(f"provider native digest failed: {error}") from error
