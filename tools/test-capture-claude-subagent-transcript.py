#!/usr/bin/env python3
"""No-provider checks for the native Claude subagent evidence helper."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "tools/capture-claude-subagent-transcript.py"
SPEC = importlib.util.spec_from_file_location("subagent_capture", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def test_dry_run_is_non_live_and_bounded() -> None:
    output = ROOT / "fixtures/public-driver-transcripts/capture-subagent-test.jsonl"
    result = subprocess.run([sys.executable, str(SCRIPT), "--model", "haiku", "--output", str(output), "--dry-run"], text=True, capture_output=True, check=False)
    assert result.returncode == 0, result.stderr
    plan = json.loads(result.stdout)
    assert plan["rawOutput"] == "bounded-memory-only"
    assert "--tools" in plan["command"] and "Task" in plan["command"]
    assert "--allowedTools" in plan["command"] and "Task(gent_probe)" in plan["command"]
    assert "--dangerously-skip-permissions" not in plan["command"]
    assert not output.exists()


def test_requires_correlated_native_agent_facts() -> None:
    observed = "\n".join((
        '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"agent-1"}]}}',
        '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"agent-1"}]}}',
        '{"type":"result","subtype":"success","is_error":false}',
    ))
    assert MODULE.observed_native_subagent(observed)
    assert not MODULE.observed_native_subagent(observed.replace('"agent-1"}]', '"other"}]', 1))
    assert not MODULE.observed_native_subagent(observed.replace('"is_error":false', '"is_error":true'))


def main() -> None:
    test_dry_run_is_non_live_and_bounded()
    test_requires_correlated_native_agent_facts()
    print("Claude subagent capture checks passed")


if __name__ == "__main__":
    main()
