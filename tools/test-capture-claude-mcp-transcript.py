#!/usr/bin/env python3
"""No-provider checks for the bounded Claude MCP evidence helper."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "tools/capture-claude-mcp-transcript.py"
SPEC = importlib.util.spec_from_file_location("mcp_capture", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def test_dry_run_is_bounded_and_local() -> None:
    output = ROOT / "fixtures/public-driver-transcripts/capture-mcp-test.jsonl"
    result = subprocess.run([sys.executable, str(SCRIPT), "--model", "haiku", "--output", str(output), "--dry-run"], text=True, capture_output=True, check=False)
    assert result.returncode == 0, result.stderr
    plan = json.loads(result.stdout)
    assert plan["rawOutput"] == "bounded-memory-only"
    assert "--strict-mcp-config" in plan["command"]
    assert MODULE.NATIVE_TOOL_NAME in plan["command"]
    assert "--safe-mode" not in plan["command"]
    assert "--dangerously-skip-permissions" not in plan["command"]
    assert not output.exists()


def test_requires_correlated_configured_mcp_facts() -> None:
    observed = "\n".join((
        '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__gent_probe__gent_probe_ping","id":"probe-1"}]}}',
        '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"probe-1"}]}}',
        '{"type":"result","subtype":"success","is_error":false}',
    ))
    assert MODULE.observed_mcp_tool(observed)
    assert not MODULE.observed_mcp_tool(observed.replace("gent_probe_ping", "other", 1))
    assert not MODULE.observed_mcp_tool(observed.replace('"probe-1"}]', '"other"}]', 1))


def main() -> None:
    test_dry_run_is_bounded_and_local()
    test_requires_correlated_configured_mcp_facts()
    print("Claude MCP capture checks passed")


if __name__ == "__main__":
    main()
