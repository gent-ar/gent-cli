#!/usr/bin/env python3
import argparse
import json
import pathlib
import sys

from provider_pins import PINS, load_pins, pinned_entries


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Print the unsigned compatibility and package-policy entries that pins.json requires")
    parser.add_argument("--node-runtime-digest", required=True, help="sha256 of the packaged Node binary the release stages")
    parser.add_argument("--terms-version", required=True)
    parser.add_argument("--pins", type=pathlib.Path, default=PINS)
    return parser.parse_args()


def main() -> int:
    args = arguments()
    print(json.dumps(pinned_entries(load_pins(args.pins), args.node_runtime_digest, args.terms_version), indent=2))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        raise SystemExit(f"provider authority entries failed: {error}") from error
