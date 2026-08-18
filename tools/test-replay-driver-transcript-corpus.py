#!/usr/bin/env python3
"""No-provider checks for the sanitized development corpus replayer."""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "tools/replay-driver-transcript-corpus.py"
SPEC = importlib.util.spec_from_file_location("corpus_replayer", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


def write_scenario(root: Path, events: list[dict[str, object]]) -> Path:
    scenario = root / "codex" / "terminal-replay"
    scenario.mkdir(parents=True)
    (scenario / "manifest.json").write_text(json.dumps({
        "format": "gent-driver-transcript-v1", "provider": "codex",
        "scenario": "terminal-replay", "source": "synthetic",
        "recordedAt": "2026-08-18T00:00:00Z", "reviewedAt": "2026-08-18T00:00:01Z",
        "notes": "safe offline replay record",
    }), encoding="utf-8")
    (scenario / "events.jsonl").write_text(
        "\n".join(json.dumps(event) for event in events) + "\n", encoding="utf-8"
    )
    return root


def test_committed_corpus_replays_offline_without_event_text() -> None:
    summaries = MODULE.replay_corpus(ROOT / "drivers_transcript")
    assert len(summaries) == 3
    assert {summary.provider for summary in summaries} == {"claude", "codex"}
    assert all(summary.event_count == 1 for summary in summaries)
    assert all(summary.terminal_outcomes == ("completed",) for summary in summaries)


def test_replay_preserves_normalized_order_and_counts() -> None:
    with tempfile.TemporaryDirectory() as directory:
        events = [
            {"sequence": 1, "type": "activity", "data": {"state": "thinking"}},
            {"sequence": 2, "type": "terminal", "data": {"outcome": "completed"}},
        ]
        summary = MODULE.replay_corpus(write_scenario(Path(directory), events))[0]
        assert summary.event_count == 2
        assert summary.event_types == ("activity", "terminal")
        assert summary.terminal_outcomes == ("completed",)


def test_terminal_without_an_outcome_is_not_replayable() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = write_scenario(Path(directory), [
            {"sequence": 1, "type": "terminal", "data": {}},
        ])
        try:
            MODULE.replay_corpus(root)
        except ValueError as error:
            assert "requires a non-empty outcome" in str(error)
        else:
            raise AssertionError("replayer accepted terminal event without an outcome")


def main() -> None:
    test_committed_corpus_replays_offline_without_event_text()
    test_replay_preserves_normalized_order_and_counts()
    test_terminal_without_an_outcome_is_not_replayable()
    print("driver transcript corpus replay checks passed")


if __name__ == "__main__":
    main()
