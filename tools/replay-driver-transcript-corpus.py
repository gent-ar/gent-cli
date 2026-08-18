#!/usr/bin/env python3
"""Replay sanitized development transcript records without contacting a provider.

This is intentionally a narrow offline reducer. It checks the committed corpus
ordering and emits only scenario summaries, never captured text or event data.
It does not establish provider evidence or enable any daemon capability.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
from collections import Counter
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ROOT = ROOT / "drivers_transcript"
VALIDATOR_PATH = ROOT / "tools/validate-driver-transcript-corpus.py"
SPEC = importlib.util.spec_from_file_location("driver_corpus_validator", VALIDATOR_PATH)
assert SPEC is not None and SPEC.loader is not None
VALIDATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATOR)


@dataclass(frozen=True)
class ReplaySummary:
    provider: str
    scenario: str
    event_count: int
    event_types: tuple[str, ...]
    terminal_outcomes: tuple[str, ...]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", type=Path, default=DEFAULT_ROOT)
    args = parser.parse_args()
    summaries = replay_corpus(args.root.resolve())
    print(json.dumps([summary.__dict__ for summary in summaries], separators=(",", ":")))


def replay_corpus(root: Path) -> list[ReplaySummary]:
    VALIDATOR.validate_corpus(root)
    summaries = []
    for provider in sorted(path for path in root.iterdir() if path.is_dir()):
        for scenario in sorted(path for path in provider.iterdir() if path.is_dir()):
            summaries.append(replay_scenario(provider.name, scenario))
    return summaries


def replay_scenario(provider: str, scenario: Path) -> ReplaySummary:
    events = [json.loads(line) for line in (scenario / "events.jsonl").read_text(encoding="utf-8").splitlines()]
    types = Counter(event["type"] for event in events)
    outcomes = []
    for event in events:
        if event["type"] != "terminal":
            continue
        outcome = event["data"].get("outcome")
        if not isinstance(outcome, str) or not outcome:
            raise ValueError(f"{provider}/{scenario.name} terminal event requires a non-empty outcome")
        outcomes.append(outcome)
    return ReplaySummary(
        provider=provider,
        scenario=scenario.name,
        event_count=len(events),
        event_types=tuple(sorted(types)),
        terminal_outcomes=tuple(outcomes),
    )


if __name__ == "__main__":
    main()
