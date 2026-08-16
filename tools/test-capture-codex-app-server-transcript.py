#!/usr/bin/env python3
"""Deterministic no-provider checks for the Codex app-server capture harness."""
from __future__ import annotations

import importlib.util
import io
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "tools/capture-codex-app-server-transcript.py"
SPEC = importlib.util.spec_from_file_location("codex_app_capture", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class Input(io.StringIO):
    def __init__(self) -> None: super().__init__(); self.requests: list[dict[str, object]] = []
    def write(self, value: str) -> int:
        self.requests.append(json.loads(value)); return super().write(value)


class Process:
    def __init__(self, events: list[dict[str, object]], stderr: str = "", exit_status: int = 0) -> None:
        self.stdin, self.stdout, self.stderr = Input(), io.StringIO("".join(json.dumps(event) + "\n" for event in events)), io.StringIO(stderr)
        self._alive, self.exit_status = True, exit_status
    def poll(self) -> int | None: return None if self._alive else self.exit_status
    def terminate(self) -> None: self._alive = False


def fake_popen(process: Process):
    def create(*_args: object, **_kwargs: object) -> Process: return process
    return create


def test_json_rpc_correlation_and_interrupt_shape() -> None:
    process = Process([
        {"id": 1, "result": {}},
        {"id": 2, "result": {"thread": {"id": "thread-1"}}},
        {"id": 3, "result": {"turn": {"id": "turn-1"}}},
        {"method": "item/started", "params": {"item": {"type": "commandExecution"}}},
        {"method": "turn/completed", "params": {"turn": {"status": "interrupted"}}},
    ])
    seen = MODULE.capture(Path("/fake/codex"), "interrupt", "gpt-5.6-luna", timeout=1, popen=fake_popen(process))
    methods = [request["method"] for request in process.stdin.requests if "method" in request and "id" in request]
    assert methods == ["initialize", "thread/start", "turn/start", "turn/interrupt"]
    interrupt = process.stdin.requests[-1]["params"]
    assert interrupt == {"threadId": "thread-1", "turnId": "turn-1"}
    assert MODULE.required("interrupt", seen) == {"turn/completed"}


def test_server_request_is_declined_with_the_same_json_rpc_id() -> None:
    process = Process([
        {"id": 1, "result": {}}, {"id": 2, "result": {"thread": {"id": "thread-1"}}},
        {"id": 3, "result": {"turn": {"id": "turn-1"}}},
        {"id": "approval-1", "method": "item/commandExecution/requestApproval", "params": {}},
        {"method": "turn/completed", "params": {"turn": {"status": "completed"}}},
    ])
    seen = MODULE.capture(Path("/fake/codex"), "permission_prompt", "gpt-5.6-luna", timeout=1, popen=fake_popen(process))
    replies = [request for request in process.stdin.requests if request.get("id") == "approval-1"]
    assert replies == [{"jsonrpc": "2.0", "id": "approval-1", "result": {"decision": "decline"}}]
    assert MODULE.required("permission_prompt", seen) == {"item/commandExecution/requestApproval"}


def test_live_consent_and_dry_run_are_safe() -> None:
    output = ROOT / "fixtures/public-driver-transcripts/codex-app-server-test.jsonl"
    result = subprocess.run([sys.executable, str(SCRIPT), "steer", "--model", "gpt-5.6-luna", "--output", str(output), "--dry-run"], text=True, capture_output=True, check=False)
    assert result.returncode == 0, result.stderr
    plan = json.loads(result.stdout)
    assert plan["manifest"] == "unchanged" and plan["command"][0].startswith("<codex")
    denied = subprocess.run([sys.executable, str(SCRIPT), "steer", "--model", "gpt-5.6-luna", "--output", str(output)], text=True, capture_output=True, check=False)
    assert denied.returncode == 2 and "confirm-live-capture" in denied.stderr
    assert not output.exists()


def test_missing_or_wrong_structural_conditions_never_validate() -> None:
    permission = [{"method": "item/commandExecution/requestApproval", "params": {}},
                  {"method": "item/completed", "params": {"item": {"type": "commandExecution"}}},
                  {"method": "item/completed", "params": {"item": {"type": "commandExecution"}}},
                  {"method": "turn/completed", "params": {"turn": {"status": "completed"}}}]
    assert MODULE.required("permission_persistent", permission)
    permission.append({"method": "item/commandExecution/requestApproval", "params": {}})
    assert not MODULE.required("permission_persistent", permission)
    assert not MODULE.required("interrupt", [{"method": "turn/completed", "params": {"turn": {"status": "completed"}}}])
    assert MODULE.frames("mcp_tool", {"item/mcpToolCall/progress"})[0]["in"]["nativeType"] == "item/mcpToolCall/progress"
    assert MODULE.registered_mcp({"data": [{"name": "isolated", "tools": {"gent_probe": {}}}]}, "isolated")
    assert not MODULE.registered_mcp({"data": [{"name": "isolated", "tools": {}}]}, "isolated")
    plan = {"method": "thread/settings/updated", "params": {"threadSettings": {"collaborationMode": {"mode": "plan"}}}}
    assert MODULE.required("plan_mode", [plan]) == {"thread/settings/updated"}
    assert not MODULE.required("plan_mode", [{"method": "thread/settings/updated", "params": {}}])
    compaction = {"method": "item/completed", "params": {"item": {"type": "contextCompaction"}}}
    assert MODULE.required("compaction", [compaction]) == {"item/completed"}


def test_stderr_diagnostic_is_generic_and_redacted() -> None:
    process = Process([], "authentication failed: token=super-secret", 23); process.terminate()
    session = MODULE.Session(process, 1); session.thread.join(1); session.stderr_thread.join(1)
    diagnostic = session.diagnostic()
    assert "exit=23,stderr=authentication" in diagnostic
    assert "super-secret" not in diagnostic and "token=" not in diagnostic


def test_replay_plan_reports_manifest_state_without_side_effects() -> None:
    args = type(
        "Args",
        (),
        {
            "scenario": "permission_prompt",
            "output": Path(
                "/Users/ivanmatiasfort/Clouseau/gent-cli/fixtures/public-driver-transcripts/candidate.jsonl"
            ),
        },
    )
    manifest, updated = MODULE.manifest_update(args.scenario, args.output.name, True)
    assert manifest == MODULE.ROOT / "fixtures/public-driver-transcripts/manifest.yml"
    assert "scenario: permission_prompt, state: recorded, path: candidate.jsonl" in updated
    assert "vendor: codex, scenario: permission_prompt, state: capture_required" not in updated


def main() -> None:
    test_json_rpc_correlation_and_interrupt_shape()
    test_server_request_is_declined_with_the_same_json_rpc_id()
    test_live_consent_and_dry_run_are_safe()
    test_missing_or_wrong_structural_conditions_never_validate()
    test_stderr_diagnostic_is_generic_and_redacted()
    print("Codex app-server capture checks passed")


if __name__ == "__main__": main()
