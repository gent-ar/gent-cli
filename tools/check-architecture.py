#!/usr/bin/env python3
"""Enforce Gent's crate dependency law and the 300-line source-file limit."""

import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
ALLOWED = {
    "gent-types": set(),
    "gent-ports": {"gent-types"},
    "gent-core": {"gent-types", "gent-ports"},
    "gent-protocol": {"gent-types"},
    "gent-adapters": {"gent-types", "gent-ports"},
    "gent-store": {"gent-types", "gent-ports"},
    "gent-drivers": {"gent-types", "gent-ports"},
    "gent-git": {"gent-types", "gent-ports"},
    "gent-mcp": {"gent-types", "gent-ports"},
    "gent-automations": {"gent-types", "gent-ports"},
    "gent-pairing": {"gent-types", "gent-protocol"},
    "gent-runtime": {"gent-types", "gent-ports", "gent-core", "gent-protocol", "gent-adapters", "gent-drivers"},
    "gent-testkit": {"gent-types", "gent-ports", "gent-protocol"},
    "gentd": None,
    "gent-cli": {"gent-types", "gent-protocol"},
}


def check_dependencies() -> list[str]:
    data = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ROOT))
    errors = []
    for package in data["packages"]:
        name = package["name"]
        if name not in ALLOWED:
            continue
        if ALLOWED[name] is None:
            continue
        direct = {dep["name"] for dep in package["dependencies"] if dep["name"].startswith("gent-") and dep.get("kind") is None}
        illegal = direct - ALLOWED[name]
        if illegal:
            errors.append(f"{name} illegally depends on {', '.join(sorted(illegal))}")
    return errors


def check_file_lengths() -> list[str]:
    errors = []
    for source in ROOT.glob("crates/**/src/**/*.rs"):
        count = len(source.read_text().splitlines())
        if count > 300:
            errors.append(f"{source.relative_to(ROOT)} has {count} lines (maximum is 300)")
    return errors


errors = check_dependencies() + check_file_lengths()
if errors:
    print("architecture check failed:", *errors, sep="\n- ", file=sys.stderr)
    sys.exit(1)
print("architecture and source-size checks passed")
