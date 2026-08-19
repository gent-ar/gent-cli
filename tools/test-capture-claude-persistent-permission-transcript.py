#!/usr/bin/env python3
"""No-provider checks for the bounded Claude persistent-permission recorder."""

from __future__ import annotations

import importlib.util
import io
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

sys.path.insert(0, str(ROOT / "tools"))
from public_driver_capture_permission import PersistentPermissionReader  # noqa: E402


def test_dry_run_does_not_start_provider() -> None:
    output = ROOT / "fixtures/public-driver-transcripts/capture-permission-test.jsonl"
    result = subprocess.run([sys.executable, str(SCRIPT), "--model", "haiku", "--output", str(output), "--dry-run"], text=True, capture_output=True, check=False)
    assert result.returncode == 0, result.stderr
    plan = json.loads(result.stdout)
    assert plan["approval"] == "exact disposable command only"
    assert plan["rawOutput"] == "bounded-memory-only"
    command = MODULE.command(Path("<claude>"), "haiku")
    # The real relay Claude Code's own protocol uses on every app session — never an
    # external MCP approval server.
    index = command.index("--permission-prompt-tool")
    assert command[index + 1] == "stdio"
    assert "--strict-mcp-config" not in command
    assert "--mcp-config" not in command
    assert "--dangerously-skip-permissions" not in command
    assert not output.exists()


def test_reader_confirms_persistence_without_a_second_prompt() -> None:
    expected = "mkdir -p /tmp/gent-evidence/approved"
    lines = [
        json.dumps({"type": "control_request", "request_id": "r1", "request": {
            "subtype": "can_use_tool", "tool_name": "Bash",
            "input": {"command": expected},
            "permission_suggestions": [{"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": expected}], "behavior": "allow"}],
        }}),
        json.dumps({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "name": "Bash", "id": "one", "input": {"command": expected}},
        ]}}),
        json.dumps({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "name": "Bash", "id": "two", "input": {"command": expected}},
        ]}}),
        json.dumps({"type": "result", "subtype": "success", "is_error": False}),
    ]
    reader = PersistentPermissionReader(1 << 20, expected)
    reader.drain(io.BytesIO(("\n".join(lines) + "\n").encode()))
    assert reader.approval_requested.is_set()
    assert reader.second_call_seen.is_set()
    assert reader.terminal.is_set()
    assert not reader.reprompted.is_set()
    assert reader.request is not None
    suggestions = reader.request["request"]["permission_suggestions"]
    assert suggestions[0]["type"] == "addRules"


def test_reader_flags_a_second_prompt_as_non_persistence() -> None:
    expected = "mkdir -p /tmp/gent-evidence/approved"
    request = {"type": "control_request", "request_id": "r1", "request": {
        "subtype": "can_use_tool", "tool_name": "Bash", "input": {"command": expected},
    }}
    lines = [json.dumps(request), json.dumps({**request, "request_id": "r2"})]
    reader = PersistentPermissionReader(1 << 20, expected)
    reader.drain(io.BytesIO(("\n".join(lines) + "\n").encode()))
    assert reader.approval_requested.is_set()
    assert reader.reprompted.is_set()


def test_reader_ignores_an_unrelated_command() -> None:
    expected = "mkdir -p /tmp/gent-evidence/approved"
    other = json.dumps({"type": "control_request", "request_id": "r1", "request": {
        "subtype": "can_use_tool", "tool_name": "Bash", "input": {"command": "rm -rf /"},
    }})
    reader = PersistentPermissionReader(1 << 20, expected)
    reader.drain(io.BytesIO((other + "\n").encode()))
    assert not reader.approval_requested.is_set()


def main() -> None:
    test_dry_run_does_not_start_provider()
    test_reader_confirms_persistence_without_a_second_prompt()
    test_reader_flags_a_second_prompt_as_non_persistence()
    test_reader_ignores_an_unrelated_command()
    print("Claude persistent-permission capture checks passed")


if __name__ == "__main__": main()
