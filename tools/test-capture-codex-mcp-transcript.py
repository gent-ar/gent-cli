#!/usr/bin/env python3
"""No-provider checks for the isolated Codex MCP transcript helper."""

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "tools/capture-codex-mcp-transcript.py"
SPEC = importlib.util.spec_from_file_location("codex_mcp_capture", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def test_probe_config_has_one_fixed_stdio_tool() -> None:
    overrides = MODULE.config_overrides()
    assert overrides[0] == "mcp_servers = {}"
    assert len(overrides) == 3
    assert all("mcp_servers.gent_probe" in item for item in overrides[1:])
    assert "--serve" in overrides[2]
    assert "http" not in " ".join(overrides)


def test_dry_run_requires_an_explicit_isolated_home() -> None:
    output = ROOT / "fixtures/public-driver-transcripts/codex-mcp-test.jsonl"
    environment = {**os.environ, "CODEX_HOME": "/tmp/gent-codex-isolated-test"}
    result = subprocess.run(
        [
            sys.executable,
            str(SCRIPT),
            "--model",
            "gpt-5.6-luna",
            "--output",
            str(output),
            "--dry-run",
            "--timeout-seconds",
            "1",
        ],
        text=True,
        capture_output=True,
        check=False,
        env=environment,
    )
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)["scenario"] == "mcp_tool"
    assert not output.exists()


def test_missing_isolated_home_cannot_start_a_capture() -> None:
    environment = {key: value for key, value in os.environ.items() if key != "CODEX_HOME"}
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--model", "gpt-5.6-luna", "--output", "ignored.jsonl"],
        text=True,
        capture_output=True,
        check=False,
        env=environment,
    )
    assert result.returncode == 2
    assert "isolated authenticated Codex home" in result.stderr


def test_capture_deadline_terminates_its_child() -> None:
    try:
        MODULE.run_capture([sys.executable, "-c", "import time; time.sleep(60)"], 1)
    except ValueError as error:
        assert "bounded deadline" in str(error)
    else:
        raise AssertionError("a stalled capture must not outlive its deadline")


def main() -> None:
    test_probe_config_has_one_fixed_stdio_tool()
    test_dry_run_requires_an_explicit_isolated_home()
    test_missing_isolated_home_cannot_start_a_capture()
    test_capture_deadline_terminates_its_child()
    print("Codex MCP capture checks passed")


if __name__ == "__main__":
    main()
