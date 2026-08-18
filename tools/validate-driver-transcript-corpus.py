#!/usr/bin/env python3
"""Validate the committed, development-only Gent driver transcript corpus."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ROOT = ROOT / "drivers_transcript"
PROVIDERS = {"claude", "codex", "claurst"}
REQUIRED_MANIFEST = {
    "format", "provider", "scenario", "source", "recordedAt", "reviewedAt", "notes",
}
ALLOWED_MANIFEST = REQUIRED_MANIFEST | {"attachments"}
EVENT_TYPES = {
    "conversation", "run", "turn", "message", "activity", "plan", "attachment", "terminal",
}
BANNED_KEY_PARTS = {
    "apikey", "authorization", "cookie", "credential", "endpoint", "environment", "env",
    "nativeframe", "password", "providerframe", "providersession", "rawframe", "resumeid",
    "routing", "secret", "sessionid", "token",
}
SECRET_PATTERNS = (
    ("OpenAI or Anthropic key prefix", re.compile(r"\bsk-[a-z0-9_-]+", re.I)),
    ("GitHub token prefix", re.compile(r"\b(?:ghp|github_pat)_[a-z0-9_]+", re.I)),
    ("bearer credential marker", re.compile(r"\bbearer\s+\S+", re.I)),
    ("API key marker", re.compile(r"\bapi[_-]?key\b", re.I)),
    ("absolute source path", re.compile(r"(?:^|\s)(?:/Users/|/home/|[A-Z]:\\Users\\)")),
)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", type=Path, default=DEFAULT_ROOT)
    args = parser.parse_args()
    try:
        validate_corpus(args.root.resolve())
    except ValueError as error:
        raise SystemExit(f"driver transcript corpus invalid: {error}") from error
    print("driver transcript corpus checks passed")


def validate_corpus(root: Path) -> None:
    root = root.resolve()
    if not root.is_dir() or root.is_symlink():
        raise ValueError("corpus root must be a regular directory")
    for path in root.rglob("*"):
        ensure_inside(root, path)
        if path.is_symlink():
            raise ValueError(f"symlink is not allowed: {relative(root, path)}")
    scenarios = [path for path in root.iterdir() if path.is_dir() and not path.name.startswith(".")]
    for provider in scenarios:
        if provider.name not in PROVIDERS:
            raise ValueError(f"unknown provider directory: {provider.name}")
        for scenario in provider.iterdir():
            if scenario.is_dir():
                validate_scenario(root, provider.name, scenario)
            elif scenario.name != "README.md":
                raise ValueError(f"unexpected provider file: {relative(root, scenario)}")
    top_files = [path.name for path in root.iterdir() if path.is_file() and path.name != "README.md"]
    if top_files:
        raise ValueError(f"unexpected corpus file: {sorted(top_files)[0]}")


def validate_scenario(root: Path, provider: str, scenario: Path) -> None:
    files = {path.name: path for path in scenario.iterdir()}
    if set(files) != {"manifest.json", "events.jsonl"}:
        raise ValueError(f"{relative(root, scenario)} must contain only manifest.json and events.jsonl")
    manifest = load_json(files["manifest.json"])
    if not isinstance(manifest, dict) or set(manifest) - ALLOWED_MANIFEST or REQUIRED_MANIFEST - set(manifest):
        raise ValueError(f"{relative(root, files['manifest.json'])} has an invalid manifest shape")
    if manifest["format"] != "gent-driver-transcript-v1" or manifest["provider"] != provider:
        raise ValueError(f"{relative(root, files['manifest.json'])} has an invalid format or provider")
    for key in REQUIRED_MANIFEST - {"format", "provider"}:
        if not isinstance(manifest[key], str) or not manifest[key].strip():
            raise ValueError(f"{relative(root, files['manifest.json'])} field {key} must be a non-empty string")
    validate_value(manifest, relative(root, files["manifest.json"]))
    lines = files["events.jsonl"].read_text(encoding="utf-8").splitlines()
    if not lines:
        raise ValueError(f"{relative(root, files['events.jsonl'])} must not be empty")
    for sequence, line in enumerate(lines, start=1):
        try:
            event = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(f"{relative(root, files['events.jsonl'])}:{sequence} is not JSON") from error
        if not isinstance(event, dict) or set(event) - {"sequence", "type", "data"}:
            raise ValueError(f"{relative(root, files['events.jsonl'])}:{sequence} has an invalid event shape")
        if event.get("sequence") != sequence or event.get("type") not in EVENT_TYPES or not isinstance(event.get("data"), dict):
            raise ValueError(f"{relative(root, files['events.jsonl'])}:{sequence} has invalid event metadata")
        validate_value(event, f"{relative(root, files['events.jsonl'])}:{sequence}")


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise ValueError(f"{path.name} is not JSON") from error


def validate_value(value: Any, location: str) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            normalized = key.lower().replace("_", "").replace("-", "")
            if any(part in normalized for part in BANNED_KEY_PARTS):
                raise ValueError(f"{location} contains a forbidden field: {key}")
            validate_value(child, location)
    elif isinstance(value, list):
        for child in value:
            validate_value(child, location)
    elif isinstance(value, str):
        for rule, pattern in SECRET_PATTERNS:
            if pattern.search(value):
                raise ValueError(f"{location} contains possible secret or source path ({rule})")


def ensure_inside(root: Path, path: Path) -> None:
    if root not in path.resolve().parents and path.resolve() != root:
        raise ValueError("corpus path escapes its root")


def relative(root: Path, path: Path) -> str:
    return str(path.relative_to(root))


if __name__ == "__main__":
    main()
