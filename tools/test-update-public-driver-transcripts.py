#!/usr/bin/env python3
"""Provider-free checks for transcript refresh planning and gap reporting."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import subprocess
import sys
import tempfile
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


def plan(manifest_body: str, *arguments: str) -> tuple[int, str]:
    with tempfile.TemporaryDirectory() as directory:
        manifest = Path(directory) / "manifest.yml"
        manifest.write_text(manifest_body, encoding="utf-8")
        previous_manifest, previous_argv = MODULE.MANIFEST, sys.argv
        MODULE.MANIFEST, sys.argv = manifest, ["update-public-driver-transcripts.py", *arguments]
        output = io.StringIO()
        try:
            with contextlib.redirect_stdout(output):
                code = MODULE.main()
        finally:
            MODULE.MANIFEST, sys.argv = previous_manifest, previous_argv
    return code, output.getvalue()


def test_gap_reports_external_prerequisite_without_a_live_call() -> None:
    code, output = plan(
        "cells:\n  - { vendor: claude, scenario: subagent, state: capture_required }\n",
        "--vendor",
        "claude",
    )
    assert code == 1
    assert "Capture prerequisites:" in output
    assert MODULE.CAPTURE_PREREQUISITE in output
    assert "confirm-live-capture" not in output


def test_claude_compaction_has_a_bounded_capture_command() -> None:
    command = MODULE.command_for("claude", "compaction", "haiku")
    assert command is not None
    assert "tools/capture-public-driver-transcript.py" in command
    assert command[command.index("--model") + 1] == "haiku"


def test_run_rejects_mixed_unsupported_request_before_any_capture() -> None:
    code, output = plan(
        "cells:\n"
        "  - { vendor: claude, scenario: full_turn, state: capture_required }\n"
        "  - { vendor: claude, scenario: subagent, state: capture_required }\n",
        "--vendor",
        "claude",
        "--run",
        "--confirm",
    )
    assert code == 1
    assert "No live capture was invoked" in output
    assert "claude:subagent" in output


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


def test_the_shipped_corpus_has_no_outstanding_capture_gap() -> None:
    result = run()
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == "No unrecorded cells match this query."


def main() -> None:
    test_gap_reports_external_prerequisite_without_a_live_call()
    test_run_rejects_mixed_unsupported_request_before_any_capture()
    test_recorded_mcp_cell_is_not_replayed_without_a_new_gap()
    test_claude_compaction_has_a_bounded_capture_command()
    test_codex_subagent_has_a_documented_bounded_capture_command()
    test_the_shipped_corpus_has_no_outstanding_capture_gap()
    print("transcript refresh planning checks passed")


if __name__ == "__main__":
    main()
