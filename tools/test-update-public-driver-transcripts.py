#!/usr/bin/env python3
"""Provider-free checks for transcript refresh planning and gap reporting."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "tools/update-public-driver-transcripts.py"


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


def test_codable_mcp_cell_still_prints_its_isolated_capture_command() -> None:
    result = run("--vendor", "codex", "--scenario", "mcp_tool")
    assert result.returncode == 0, result.stderr
    assert "capture-codex-mcp-transcript.py" in result.stdout
    assert "--dry-run" in result.stdout


def test_claude_persistent_permission_has_a_bounded_capture_command() -> None:
    result = run("--vendor", "claude", "--scenario", "permission_persistent")
    assert result.returncode == 0, result.stderr
    assert "capture-claude-persistent-permission-transcript.py" in result.stdout


def main() -> None:
    test_gap_reports_external_prerequisite_without_a_live_call()
    test_run_rejects_mixed_unsupported_request_before_any_capture()
    test_codable_mcp_cell_still_prints_its_isolated_capture_command()
    print("transcript refresh planning checks passed")


if __name__ == "__main__":
    main()
