#!/usr/bin/env python3
"""No-provider regression checks for the public-driver capture helper."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "tools/capture-public-driver-transcript.py"
SPEC = importlib.util.spec_from_file_location("capture_tool", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def command(*arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run([sys.executable, str(SCRIPT), *arguments], text=True,
                          capture_output=True, check=False)


def output(name: str) -> str:
    return str(ROOT / "fixtures/public-driver-transcripts" / name)


def test_dry_run_never_requires_a_provider_or_writes() -> None:
    candidate = ROOT / "fixtures/public-driver-transcripts/capture-test-candidate.jsonl"
    assert not candidate.exists()
    result = command("claude", "full_turn", "--model", "haiku", "--output", str(candidate), "--dry-run")
    assert result.returncode == 0, result.stderr
    plan = json.loads(result.stdout)
    assert plan["command"][0] == "<claude-resolved-at-live-capture>"
    assert plan["rawOutput"] == "bounded-memory-only"
    assert plan["attestation"] == "redacted_normalized_fixture_v1"
    assert not candidate.exists()


def test_public_tool_refuses_claurst_and_escape_paths() -> None:
    claurst = command("claurst", "full_turn", "--model", "safe", "--output", output("x.jsonl"), "--dry-run")
    assert claurst.returncode != 0
    assert "invalid choice" in claurst.stderr
    escaped = command("codex", "full_turn", "--model", "safe", "--output", "../x.jsonl", "--dry-run")
    assert escaped.returncode == 2
    assert "fixtures/public-driver-transcripts" in escaped.stderr


def test_attestation_only_covers_reviewed_normalized_facts() -> None:
    metadata = {"vendor": "claude", "scenario": "full_turn", "attestationScope": "redacted_normalized_fixture_v1"}
    frames = MODULE.normalized_frames("claude", "full_turn")
    first = MODULE.attestation(metadata, frames)
    assert first == MODULE.attestation({**metadata, "attestationDigest": "ignored"}, frames)
    assert first != MODULE.attestation(metadata, MODULE.normalized_frames("claude", "tool_use"))


def test_thinking_capture_requires_an_observed_vendor_signal() -> None:
    assert MODULE.scenario_was_observed("codex", "thinking", '{"type":"turn.started"}')
    assert not MODULE.scenario_was_observed("codex", "thinking", '{"type":"turn.completed"}')
    assert MODULE.scenario_was_observed("claude", "thinking", '{"type":"thinking"}')


def test_permission_capture_requires_a_manual_request_and_denial() -> None:
    stream = "\n".join((
        '{"type":"assistant","message":{"content":[{"type":"tool_use"}]}}',
        '{"type":"result","permission_denials":["Bash"]}',
    ))
    assert MODULE.scenario_was_observed("claude", "permission_prompt", stream)
    assert not MODULE.scenario_was_observed("codex", "permission_prompt", stream)
    assert not MODULE.scenario_was_observed("claude", "permission_prompt", stream.rsplit("\n", 1)[0])


def test_manifest_replacement_is_prepared_without_writing() -> None:
    args = type("Args", (), {"vendor": "claude", "scenario": "permission_prompt", "output": Path(output("candidate.jsonl"))})()
    manifest, updated = MODULE.manifest_update(args, True)
    assert manifest == ROOT / "fixtures/public-driver-transcripts/manifest.yml"
    assert "scenario: permission_prompt, state: recorded, path: candidate.jsonl" in updated
    assert "vendor: claude, scenario: permission_prompt, state: capture_required" not in updated


def main() -> None:
    test_dry_run_never_requires_a_provider_or_writes()
    test_public_tool_refuses_claurst_and_escape_paths()
    test_attestation_only_covers_reviewed_normalized_facts()
    test_thinking_capture_requires_an_observed_vendor_signal()
    test_manifest_replacement_is_prepared_without_writing()
    print("public-driver capture tool checks passed")


if __name__ == "__main__":
    main()
