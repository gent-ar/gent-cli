#!/usr/bin/env python3
"""No-provider checks for the bounded Claude persistent-permission recorder."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "tools/capture-claude-persistent-permission-transcript.py"
SPEC = importlib.util.spec_from_file_location("permission_capture", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def test_dry_run_does_not_start_provider() -> None:
    output = ROOT / "fixtures/public-driver-transcripts/capture-permission-test.jsonl"
    result = subprocess.run([sys.executable, str(SCRIPT), "--model", "haiku", "--output", str(output), "--dry-run"], text=True, capture_output=True, check=False)
    assert result.returncode == 0, result.stderr
    plan = json.loads(result.stdout)
    assert plan["approval"] == "exact disposable command only"
    assert plan["rawOutput"] == "bounded-memory-only"
    command = MODULE.command(Path("<claude>"), "haiku", "<local-config>", "mkdir -p /tmp/probe")
    assert "--permission-prompt-tool" in command
    assert "--dangerously-skip-permissions" not in command
    assert not output.exists()


def test_exact_command_and_correlated_two_call_facts_are_required() -> None:
    expected = "mkdir -p /tmp/gent-evidence/approved"
    stream = "\n".join((
        '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","id":"one","input":{"command":"mkdir -p /tmp/gent-evidence/approved"}}]}}',
        '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"one"}]}}',
        '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","id":"two","input":{"command":"mkdir -p /tmp/gent-evidence/approved"}}]}}',
        '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"two"}]}}',
        '{"type":"assistant","message":{"content":"GENT_PERSISTENT_PERMISSION_OK"}}',
        '{"type":"result","subtype":"success","is_error":false}',
    ))
    assert MODULE.permitted({"tool_name": "Bash", "input": {"command": expected}}, expected)
    assert not MODULE.permitted({"tool_name": "Bash", "input": {"command": "rm -rf /"}}, expected)
    assert MODULE.observed(stream, expected)
    assert not MODULE.observed(stream.replace('"id":"two"', '"id":"one"', 1), expected)


def main() -> None:
    test_dry_run_does_not_start_provider()
    test_exact_command_and_correlated_two_call_facts_are_required()
    print("Claude persistent-permission capture checks passed")


if __name__ == "__main__": main()
