#!/usr/bin/env python3
"""Enforce Gent's dependency, production-import, and source-size boundaries."""

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
    "gent-runtime": {"gent-types", "gent-ports", "gent-core", "gent-protocol"},
    "gent-testkit": {"gent-types", "gent-ports", "gent-protocol"},
    "gentd": None,
    "gent-cli": {"gent-types", "gent-protocol"},
}

PRODUCT_DOMAINS = {
    "gent-adapters",
    "gent-drivers",
    "gent-git",
    "gent-mcp",
    "gent-store",
}

SOURCE_SUFFIXES = {".ps1", ".py", ".rs", ".sh", ".yaml", ".yml"}
SCRIPT_NAMES = {"validate-coverage-manifest"}


def source_files() -> list[pathlib.Path]:
    """Returns checked source, test, automation, and CI files, never generated data."""
    roots = (ROOT / "crates", ROOT / "tools", ROOT / ".github" / "workflows")
    return [
        path
        for root in roots
        if root.exists()
        for path in root.rglob("*")
        if path.is_file() and (path.suffix in SOURCE_SUFFIXES or path.name in SCRIPT_NAMES)
    ]


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
    for source in source_files():
        count = len(source.read_text(encoding="utf-8").splitlines())
        if count > 300:
            errors.append(f"{source.relative_to(ROOT)} has {count} lines (maximum is 300)")
    return errors


def test_module_lines(lines: list[str]) -> set[int]:
    """Returns line indexes enclosed by a conventional `#[cfg(test)] mod` body."""
    ignored = set()
    pending = False
    depth = 0
    for index, line in enumerate(lines):
        if line.strip() == "#[cfg(test)]":
            pending = True
            continue
        if pending and line.lstrip().startswith("mod ") and "{" in line:
            pending = False
            depth = line.count("{") - line.count("}")
            ignored.add(index)
            continue
        if pending and line.strip():
            pending = False
        if depth:
            ignored.add(index)
            depth += line.count("{") - line.count("}")
    return ignored


def check_production_imports() -> list[str]:
    errors = []
    for source in (ROOT / "crates").glob("*/src/**/*.rs"):
        crate = source.relative_to(ROOT / "crates").parts[0]
        if crate == "gentd":
            continue
        lines = source.read_text(encoding="utf-8").splitlines()
        ignored = test_module_lines(lines)
        for index, line in enumerate(lines):
            if index in ignored:
                continue
            for domain in PRODUCT_DOMAINS:
                if f"{domain.replace('-', '_')}::" in line:
                    errors.append(
                        f"{source.relative_to(ROOT)} imports product domain {domain} outside gentd"
                    )
    return errors


errors = check_dependencies() + check_production_imports() + check_file_lengths()
if errors:
    print("architecture check failed:", *errors, sep="\n- ", file=sys.stderr)
    sys.exit(1)
print("architecture and source-size checks passed")
