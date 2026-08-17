#!/usr/bin/env python3
"""No-provider checks for the bounded Codex native-subagent recorder."""
from __future__ import annotations

import importlib.util
import io
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "tools/capture-codex-subagent-transcript.py"
SPEC = importlib.util.spec_from_file_location("codex_subagent_capture", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class Input(io.StringIO):
    def write(self, value: str) -> int:
        json.loads(value)
        return super().write(value)


class Process:
    def __init__(self, events: list[dict[str, object]]) -> None:
        self.stdin = Input()
        self.stdout = io.StringIO("".join(json.dumps(event) + "\n" for event in events))
        self._alive = True

    def poll(self) -> int | None:
        return None if self._alive else 0

    def terminate(self) -> None:
        self._alive = False

    def wait(self, timeout: int) -> int:
        self._alive = False
        return 0


def fake_popen(process: Process):
    def create(*_args: object, **_kwargs: object) -> Process:
        return process
    return create


def lifecycle(unsafe: str | None = None) -> list[dict[str, object]]:
    child = "child-thread"
    values: list[dict[str, object]] = [
        {"id": 1, "result": {}},
        {"id": 2, "result": {"thread": {"id": "root-thread"}}},
        {"id": 3, "result": {"turn": {"id": "turn-1"}}},
        {"method": "item/started", "params": {"item": {"type": "collabAgentToolCall", "id": "spawn",
            "tool": "spawnAgent", "status": "inProgress", "receiverThreadIds": [], "agentsStates": {}}}},
        {"method": "item/completed", "params": {"item": {"type": "collabAgentToolCall", "id": "spawn",
            "tool": "spawnAgent", "status": "completed", "receiverThreadIds": [child],
            "agentsStates": {child: {"status": "pendingInit"}}}}},
        {"method": "item/started", "params": {"item": {"type": "collabAgentToolCall", "id": "wait",
            "tool": "wait", "status": "inProgress", "receiverThreadIds": [child], "agentsStates": {}}}},
        {"method": "item/completed", "params": {"item": {"type": "collabAgentToolCall", "id": "wait",
            "tool": "wait", "status": "completed", "receiverThreadIds": [child],
            "agentsStates": {child: {"status": "completed"}}}}},
        {"method": "turn/completed", "params": {"turn": {"status": "completed"}}},
    ]
    if unsafe:
        values.insert(3, {"method": "item/started", "params": {"item": {"type": unsafe}}})
    return values


def test_correlated_lifecycle_is_required() -> None:
    assert MODULE.observed_native_subagent(lifecycle())
    missing = lifecycle()
    missing[6]["params"]["item"]["agentsStates"] = {"child-thread": {"status": "running"}}
    assert not MODULE.observed_native_subagent(missing)
    assert not MODULE.observed_native_subagent(lifecycle("commandExecution"))


def test_capture_uses_documented_ultra_effort() -> None:
    process = Process(lifecycle())
    seen = MODULE.capture(Path("/fake/codex"), "gpt-5.6-luna", 1, fake_popen(process))
    requests = [json.loads(line) for line in process.stdin.getvalue().splitlines()]
    turn = next(request for request in requests if request.get("method") == "turn/start")
    assert turn["params"]["effort"] == "ultra"
    assert MODULE.observed_native_subagent(seen)


def test_safe_cli_paths_and_normalized_frames() -> None:
    output = ROOT / "fixtures/public-driver-transcripts/codex-subagent-test.jsonl"
    dry = subprocess.run([sys.executable, str(SCRIPT), "--output", str(output), "--dry-run"],
                         text=True, capture_output=True, check=False)
    assert dry.returncode == 0, dry.stderr
    assert json.loads(dry.stdout)["effort"] == "ultra"
    denied = subprocess.run([sys.executable, str(SCRIPT), "--output", str(output)],
                            text=True, capture_output=True, check=False)
    assert denied.returncode == 2 and "confirm-live-capture" in denied.stderr
    normalized = MODULE.frames()
    assert [frame["expect"] for frame in normalized] == ["subagent_spawned", "subagent_completed"]
    assert not output.exists()


def main() -> None:
    test_correlated_lifecycle_is_required()
    test_capture_uses_documented_ultra_effort()
    test_safe_cli_paths_and_normalized_frames()
    print("Codex subagent capture checks passed")


if __name__ == "__main__":
    main()
