#!/usr/bin/env python3
"""No-provider checks for the development transcript corpus validator."""

from __future__ import annotations

import importlib.util
import json
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "tools/validate-driver-transcript-corpus.py"
SPEC = importlib.util.spec_from_file_location("corpus_validator", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def write_scenario(root: Path, event: dict[str, object] | None = None) -> Path:
    scenario = root / "codex" / "final-answer"
    scenario.mkdir(parents=True)
    (scenario / "manifest.json").write_text(json.dumps({
        "format": "gent-driver-transcript-v1", "provider": "codex",
        "scenario": "final-answer", "source": "synthetic", "recordedAt": "2026-08-17T00:00:00Z",
        "reviewedAt": "2026-08-17T00:00:00Z", "notes": "safe offline regression record",
    }), encoding="utf-8")
    (scenario / "events.jsonl").write_text(json.dumps(event or {
        "sequence": 1, "type": "message", "data": {"role": "assistant", "text": "hello"},
    }) + "\n", encoding="utf-8")
    return root


def test_valid_record_and_empty_committed_root() -> None:
    MODULE.validate_corpus(ROOT / "drivers_transcript")
    with tempfile.TemporaryDirectory() as directory:
        MODULE.validate_corpus(write_scenario(Path(directory)))


def test_secret_and_provider_session_are_rejected() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = write_scenario(Path(directory), {
            "sequence": 1, "type": "message", "data": {"text": "bearer not-a-real-token"},
        })
        assert_invalid(root, "bearer credential")
    with tempfile.TemporaryDirectory() as directory:
        root = write_scenario(Path(directory), {
            "sequence": 1, "type": "run", "data": {"providerSessionId": "forbidden"},
        })
        assert_invalid(root, "forbidden field")


def test_event_order_and_unapproved_files_are_rejected() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = write_scenario(Path(directory), {"sequence": 2, "type": "terminal", "data": {}})
        assert_invalid(root, "invalid event metadata")
    with tempfile.TemporaryDirectory() as directory:
        root = write_scenario(Path(directory))
        (root / "codex" / "final-answer" / "raw.txt").write_text("forbidden", encoding="utf-8")
        assert_invalid(root, "must contain only")


def assert_invalid(root: Path, needle: str) -> None:
    try:
        MODULE.validate_corpus(root)
    except ValueError as error:
        assert needle in str(error), error
    else:
        raise AssertionError("validator accepted an unsafe corpus")


def main() -> None:
    test_valid_record_and_empty_committed_root()
    test_secret_and_provider_session_are_rejected()
    test_event_order_and_unapproved_files_are_rejected()
    print("driver transcript corpus validator checks passed")


if __name__ == "__main__":
    main()
