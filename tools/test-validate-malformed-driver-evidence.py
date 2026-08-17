#!/usr/bin/env python3
"""Provider-free checks for malformed provider evidence validation."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "tools/validate-malformed-driver-evidence.py"
META = {
    "vendor": "claude",
    "scenario": "malformed_tolerance",
    "status": "recorded",
    "captureOrigin": "live_cli",
    "faultSource": "provider_emitted",
    "faultBoundary": "structured_provider_frame",
    "faultControlKind": "vendor_documented",
    "faultControl": "documented test output mode",
    "faultControlReference": "https://vendor.example/docs/test-output",
    "faultShapeDigest": "sha256:" + "a" * 64,
}
FAULT = {
    "in": {"type": "assistant"},
    "expect": "transport_diagnostic",
    "expectFields": {"classification": "malformedClaudeAssistant", "providerEmitted": True},
}
CONTINUATION = {
    "in": {"type": "result", "is_error": False},
    "expect": "completed_turn",
    "expectFields": {"afterFault": True},
}


def invoke(path: Path | None = None, *arguments: str) -> subprocess.CompletedProcess[str]:
    command = [sys.executable, str(SCRIPT), *arguments]
    if path is not None:
        command.append(str(path))
    return subprocess.run(command, cwd=ROOT, text=True, capture_output=True, check=False)


def fixture(directory: Path, name: str, *frames: dict[str, object], **meta: object) -> Path:
    path = directory / name
    header = {"meta": {**META, **meta}}
    path.write_text("\n".join(json.dumps(item) for item in (header, *frames)) + "\n", encoding="utf-8")
    return path


def test_describe_never_requires_a_provider() -> None:
    result = invoke(None, "--describe")
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)["source"] == "provider_emitted; never proxy, injection, replay, or shim"


def test_valid_candidate_has_explicit_live_fault_and_continuation() -> None:
    with tempfile.TemporaryDirectory() as raw:
        path = fixture(Path(raw), "candidate.jsonl", FAULT, CONTINUATION)
        result = invoke(path)
    assert result.returncode == 0, result.stderr


def test_rejects_injected_or_uncontinued_candidates() -> None:
    with tempfile.TemporaryDirectory() as raw:
        directory = Path(raw)
        injected = fixture(directory, "injected.jsonl", FAULT, CONTINUATION, faultSource="proxy_injected")
        result = invoke(injected)
        assert result.returncode != 0
        assert "faultSource" in result.stderr
        stopped = fixture(directory, "stopped.jsonl", FAULT)
        result = invoke(stopped)
        assert result.returncode != 0
        assert "continuation" in result.stderr


def test_rejects_wrong_vendor_diagnostic() -> None:
    with tempfile.TemporaryDirectory() as raw:
        path = fixture(Path(raw), "candidate.jsonl", {**FAULT, "expectFields": {"classification": "malformedCodexFrame", "providerEmitted": True}}, CONTINUATION)
        result = invoke(path)
    assert result.returncode != 0
    assert "vendor-specific" in result.stderr


def main() -> None:
    test_describe_never_requires_a_provider()
    test_valid_candidate_has_explicit_live_fault_and_continuation()
    test_rejects_injected_or_uncontinued_candidates()
    test_rejects_wrong_vendor_diagnostic()
    print("malformed provider evidence validation checks passed")


if __name__ == "__main__":
    main()
