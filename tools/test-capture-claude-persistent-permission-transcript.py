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


class FakeStdin:
    """Records every `control_response` written back, like a real process's stdin pipe."""

    def __init__(self) -> None:
        self.written: list[dict] = []

    def write(self, data: bytes) -> None:
        self.written.append(json.loads(data))

    def flush(self) -> None:
        pass


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


def control_request(request_id: str, command: str, suggestions: list | None = None) -> str:
    return json.dumps({"type": "control_request", "request_id": request_id, "request": {
        "subtype": "can_use_tool", "tool_name": "Bash", "input": {"command": command},
        "permission_suggestions": suggestions or [],
    }})


def test_reader_auto_grants_a_second_identical_request() -> None:
    expected = "mkdir -p /tmp/gent-evidence/approved"
    suggestions = [
        {"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": expected}], "destination": "localSettings"},
        {"type": "addDirectories", "directories": ["/tmp/gent-evidence"], "destination": "session"},
    ]
    lines = [
        control_request("r1", expected, suggestions),
        control_request("r2", expected, suggestions),
        json.dumps({"type": "result", "subtype": "success", "is_error": False, "permission_denials": []}),
    ]
    stdin = FakeStdin()
    reader = PersistentPermissionReader(1 << 20, expected, stdin)
    reader.drain(io.BytesIO(("\n".join(lines) + "\n").encode()))
    assert reader.first_approval.is_set()
    assert reader.second_approval.is_set()
    assert reader.terminal.is_set()
    assert reader.result is not None and not reader.result["permission_denials"]
    # Both requests were answered "allow", and the session-scoped suggestion was chosen
    # over the disk-persisted one — the CLI relays every request; this client remembers.
    assert len(stdin.written) == 2
    for response in stdin.written:
        body = response["response"]["response"]
        assert body["behavior"] == "allow"
        assert body["updatedPermissions"][0]["destination"] == "session"
    assert stdin.written[0]["response"]["request_id"] == "r1"
    assert stdin.written[1]["response"]["request_id"] == "r2"


def test_reader_ignores_an_unrelated_command() -> None:
    expected = "mkdir -p /tmp/gent-evidence/approved"
    stdin = FakeStdin()
    reader = PersistentPermissionReader(1 << 20, expected, stdin)
    reader.drain(io.BytesIO((control_request("r1", "rm -rf /") + "\n").encode()))
    assert not reader.first_approval.is_set()
    assert not stdin.written


def main() -> None:
    test_dry_run_does_not_start_provider()
    test_reader_auto_grants_a_second_identical_request()
    test_reader_ignores_an_unrelated_command()
    print("Claude persistent-permission capture checks passed")


if __name__ == "__main__": main()
