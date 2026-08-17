#!/usr/bin/env python3
"""Provider-free checks for transcript refresh planning and gap reporting."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "tools/update-public-driver-transcripts.py"
SPEC = importlib.util.spec_from_file_location("transcript_updater", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def run(*arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *arguments],
        cwd=ROOT,
        check=False,
        text=True,
        capture_output=True,
    )


def test_gap_reports_external_prerequisite_without_a_live_call() -> None:
    result = run("--vendor", "claude", "--scenario", "compaction")
    assert result.returncode == 1
    assert "Capture prerequisites:" in result.stdout
    assert "documented Claude compaction signal" in result.stdout
    assert "confirm-live-capture" not in result.stdout


def test_run_rejects_mixed_unsupported_request_before_any_capture() -> None:
    result = run("--vendor", "claude", "--run", "--confirm")
    assert result.returncode == 1
    assert "No live capture was invoked" in result.stdout
    assert "malformed_tolerance" in result.stdout


def test_recorded_mcp_cell_is_not_replayed_without_a_new_gap() -> None:
    result = run("--vendor", "codex", "--scenario", "mcp_tool")
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == "No unrecorded cells match this query."


def test_claude_persistent_permission_has_a_bounded_capture_command() -> None:
    result = run("--vendor", "claude", "--scenario", "permission_persistent")
    assert result.returncode == 0, result.stderr
    assert "capture-claude-persistent-permission-transcript.py" in result.stdout


def test_codex_subagent_has_a_documented_bounded_capture_command() -> None:
    command = MODULE.command_for("codex", "subagent", "gpt-5.6-luna")
    assert command is not None
    assert "tools/capture-codex-subagent-transcript.py" in command
    assert "--dry-run" in command


def main() -> None:
    test_gap_reports_external_prerequisite_without_a_live_call()
    test_run_rejects_mixed_unsupported_request_before_any_capture()
    test_recorded_mcp_cell_is_not_replayed_without_a_new_gap()
    test_codex_subagent_has_a_documented_bounded_capture_command()
    print("transcript refresh planning checks passed")


if __name__ == "__main__":
    main()
