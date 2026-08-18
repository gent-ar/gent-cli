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


def write_scenario(
    root: Path,
    event: dict[str, object] | list[dict[str, object]] | None = None,
    attachments: list[dict[str, object]] | None = None,
) -> Path:
    scenario = root / "codex" / "final-answer"
    scenario.mkdir(parents=True)
    manifest: dict[str, object] = {
        "format": "gent-driver-transcript-v1", "provider": "codex",
        "scenario": "final-answer", "source": "synthetic", "recordedAt": "2026-08-17T00:00:00Z",
        "reviewedAt": "2026-08-17T00:00:00Z", "notes": "safe offline regression record",
    }
    if attachments is not None:
        manifest["attachments"] = attachments
    (scenario / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    events = event if isinstance(event, list) else [event or {
        "sequence": 1, "type": "message", "data": {"role": "assistant", "text": "hello"},
    }]
    (scenario / "events.jsonl").write_text(
        "\n".join(json.dumps(item) for item in events) + "\n", encoding="utf-8"
    )
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


def test_provenance_attachment_and_raw_output_guards_are_enforced() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = write_scenario(Path(directory))
        manifest = root / "codex" / "final-answer" / "manifest.json"
        value = json.loads(manifest.read_text(encoding="utf-8"))
        value["source"] = "unknown-capture-route"
        manifest.write_text(json.dumps(value), encoding="utf-8")
        assert_invalid(root, "unsupported source")
    with tempfile.TemporaryDirectory() as directory:
        root = write_scenario(Path(directory))
        manifest = root / "codex" / "final-answer" / "manifest.json"
        value = json.loads(manifest.read_text(encoding="utf-8"))
        value["reviewedAt"] = "2026-08-16"
        manifest.write_text(json.dumps(value), encoding="utf-8")
        assert_invalid(root, "timestamp must be RFC3339")
    with tempfile.TemporaryDirectory() as directory:
        root = write_scenario(Path(directory), {
            "sequence": 1, "type": "activity", "data": {"rawOutput": "not allowed"},
        })
        assert_invalid(root, "forbidden field")
    with tempfile.TemporaryDirectory() as directory:
        root = write_scenario(Path(directory))
        manifest = root / "codex" / "final-answer" / "manifest.json"
        value = json.loads(manifest.read_text(encoding="utf-8"))
        value["attachments"] = [{"contentDigest": "sha256:" + "a" * 64,
                                 "mediaType": "image/png", "byteLength": 7}]
        manifest.write_text(json.dumps(value), encoding="utf-8")
        MODULE.validate_corpus(root)
        value["attachments"][0]["sourcePath"] = "forbidden"
        manifest.write_text(json.dumps(value), encoding="utf-8")
        assert_invalid(root, "attachment metadata is invalid")


def test_context_plan_goal_and_attachment_semantics_are_enforced() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = write_scenario(Path(directory), semantic_events("clear", 1))
        assert_invalid(root, "clear context must use historyOrdinal zero")
    with tempfile.TemporaryDirectory() as directory:
        root = write_scenario(Path(directory), semantic_events("preserve", 3))
        assert_invalid(root, "parent's frozen history ordinal")
    with tempfile.TemporaryDirectory() as directory:
        events = semantic_events("clear", 0)
        events.append({"sequence": 5, "type": "activity", "data": {
            "kind": "goal", "status": "active", "goalId": "goal-1", "revision": 1,
            "summary": "",
        }})
        root = write_scenario(Path(directory), events)
        assert_invalid(root, "summary must be a non-empty string")
    with tempfile.TemporaryDirectory() as directory:
        events = semantic_events("clear", 0)
        events.append({"sequence": 5, "type": "plan", "data": {
            "planId": "plan-1", "revision": 1, "digest": "not-a-digest", "status": "reviewed",
        }})
        root = write_scenario(Path(directory), events)
        assert_invalid(root, "plan digest")
    with tempfile.TemporaryDirectory() as directory:
        metadata = {"contentDigest": "sha256:" + "a" * 64, "mediaType": "image/png", "byteLength": 7}
        events = semantic_events("clear", 0) + [{"sequence": 5, "type": "attachment", "data": {
            **metadata, "turnId": "turn-1",
        }}]
        root = write_scenario(Path(directory), events, [metadata | {"contentDigest": "sha256:" + "b" * 64}])
        assert_invalid(root, "declared in the manifest")


def semantic_events(policy: str, history_ordinal: int) -> list[dict[str, object]]:
    return [
        {"sequence": 1, "type": "conversation", "data": {
            "conversationId": "conversation-1", "provider": "codex", "model": "gpt-5.6",
            "effort": "high", "mode": "plan",
        }},
        {"sequence": 2, "type": "run", "data": {
            "runId": "run-1", "contextPolicy": "preserve", "historyOrdinal": 2,
        }},
        {"sequence": 3, "type": "turn", "data": {"turnId": "turn-1", "ordinal": 4}},
        {"sequence": 4, "type": "run", "data": {
            "runId": "run-2", "parentRunId": "run-1", "contextPolicy": policy,
            "historyOrdinal": history_ordinal,
        }},
    ]


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
    test_provenance_attachment_and_raw_output_guards_are_enforced()
    test_context_plan_goal_and_attachment_semantics_are_enforced()
    print("driver transcript corpus validator checks passed")


if __name__ == "__main__":
    main()
