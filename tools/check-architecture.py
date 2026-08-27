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
    "gent-store": {"gent-types", "gent-ports", "gent-core"},
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

SOURCE_SUFFIXES = {".md", ".ps1", ".py", ".rs", ".sh", ".toml", ".yaml", ".yml"}
SCRIPT_NAMES = set()
GENERATED_OR_FIXTURE_ROOTS = {"fixtures", "target"}
SNAPSHOT_CONTRACTS = {
    "README.md": ("snapshot/recovery-cache/mirrored-state/replacement layer",),
    "docs/architecture.md": (
        "Snapshot, recovery-snapshot, recovery-cache,",
        "views are optional, disposable, and non-authoritative",
        "reload immutable bounded pages (from zero when the cursor is",
    ),
    "docs/realtime-agent-chat-client-plan.md": (
        "Snapshots, recovery caches, mirrored state, and state replacement are prohibited",
        "in-memory view is optional and disposable, never serialized or sent as authoritative state",
    ),
    "docs/flutter-handoff-v1.md": (
        "in-memory view is disposable and",
        "If a cursor is not accepted, reload from ordinal/cursor",
    ),
    "docs/continuation-handoff.md": (
        "Snapshot state, recovery caches, mirrored state, and replacement layers are",
        "Derived views are disposable/non-authoritative: never serialize,",
        "pages (from cursor zero when needed), then replay normalized facts",
    ),
}


def source_files() -> list[pathlib.Path]:
    """Returns tracked hand-authored files, never generated data or fixture recordings."""
    output = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT, text=True)
    return [
        ROOT / relative
        for value in output.split("\0")
        if value
        for relative in [pathlib.PurePosixPath(value)]
        if not (set(relative.parts) & GENERATED_OR_FIXTURE_ROOTS)
        and (relative.suffix in SOURCE_SUFFIXES or relative.name in SCRIPT_NAMES)
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
        direct = {
            dep["name"]
            for dep in package["dependencies"]
            if dep["name"].startswith("gent-") and dep.get("kind") != "dev"
        }
        illegal = direct - ALLOWED[name]
        if illegal:
            errors.append(f"{name} illegally depends on {', '.join(sorted(illegal))}")
    return errors


def check_file_lengths() -> list[str]:
    errors = []
    for source in source_files():
        if not source.is_file():
            continue
        lines = source.read_text(encoding="utf-8").splitlines()
        is_test_source = "tests" in source.parts or source.stem.endswith("_tests")
        ignored = set(range(len(lines))) if is_test_source else (
            test_module_lines(lines) if source.suffix == ".rs" else set()
        )
        count = sum(index not in ignored for index in range(len(lines)))
        if count > 300:
            errors.append(f"{source.relative_to(ROOT)} has {count} lines (maximum is 300)")
    return errors


def check_snapshot_contract() -> list[str]:
    """Requires the stale-state prohibition at every client boundary."""
    errors = []
    for relative, required_fragments in SNAPSHOT_CONTRACTS.items():
        contents = (ROOT / relative).read_text(encoding="utf-8")
        for fragment in required_fragments:
            if fragment not in contents:
                errors.append(f"{relative} is missing no-snapshot contract: {fragment!r}")
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


errors = (
    check_dependencies()
    + check_production_imports()
    + check_file_lengths()
    + check_snapshot_contract()
)
if errors:
    print("architecture check failed:", *errors, sep="\n- ", file=sys.stderr)
    sys.exit(1)
print("architecture and source-size checks passed")
