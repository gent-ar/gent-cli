#!/usr/bin/env python3
"""Capture a bounded, redacted Codex app-server transcript after live consent."""
from __future__ import annotations
import argparse
import datetime as dt
import hashlib
import json
import os
import re
import platform
import queue
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from pathlib import Path
from codex_mcp_probe import capture_direct, capture_probe, probe_result, registered_mcp
ROOT = Path(__file__).resolve().parent.parent; FIXTURES = ROOT / "fixtures/public-driver-transcripts"
LIMIT, TIMEOUT = 256 * 1024, 90
SCENARIOS = (
    "permission_prompt", "permission_persistent", "plan_mode", "compaction",
    "mcp_tool", "interrupt", "steer",
)
PROMPTS = {
    "permission_prompt": "Delete /tmp/gent-cli-permission-prompt-test. Do not create or edit any other files.",
    "permission_persistent": "Delete /tmp/gent-cli-permission-persistent-1 and then delete /tmp/gent-cli-permission-persistent-2 again. Do this twice in total without making any other file changes.",
    "plan_mode": "Give a two-step plan for reading a text file. Do not use tools.",
    "compaction": "Reply with the exact text GENT_COMPACTION_SEED_OK and do not use tools.",
    "mcp_tool": "Direct documented app-server call to the isolated gent_probe MCP tool.",
    "interrupt": "Run sleep 30 exactly once. Do not use any other tool.",
    "steer": "Run sleep 30 exactly once. Do not use any other tool.",
}

def args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenario", choices=SCENARIOS)
    parser.add_argument("--model", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--mcp-server", help="pre-registered isolated MCP probe server for mcp_tool")
    parser.add_argument("--codex-config", action="append", default=[], help="one isolated Codex -c TOML override")
    parser.add_argument("--timeout-seconds", type=int, default=TIMEOUT)
    parser.add_argument("--confirm-live-capture", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--replace-existing", action="store_true")
    parser.add_argument("--update-manifest", action="store_true")
    return parser.parse_args()
def output_path(path: Path) -> Path:
    resolved, root = path.resolve(), FIXTURES.resolve()
    if resolved.parent != root or resolved.suffix != ".jsonl" or path.is_symlink():
        raise ValueError("--output must be a non-symlink .jsonl directly in fixtures/public-driver-transcripts")
    return resolved
def binary() -> Path:
    found = shutil.which("codex")
    if found is None:
        raise ValueError("codex is not on PATH")
    return Path(found).resolve()
def plan(scenario: str, model: str, output: Path) -> dict[str, object]:
    if not model.replace("-", "").replace("_", "").replace(".", "").isalnum():
        raise ValueError("--model must be an identifier")
    methods = ["initialize", "thread/start", *extra_methods(scenario)]
    if scenario != "mcp_tool": methods.append("turn/start")
    return {
        "scenario": scenario, "output": str(output),
        "command": ["<codex-resolved-at-live-capture>", "app-server", "--stdio"],
        "methods": methods,
        "rawOutput": f"bounded-memory-only:{LIMIT}", "manifest": "unchanged",
    }
def extra_methods(scenario: str) -> list[str]:
    return {"permission_persistent": ["turn/start"], "compaction": ["thread/compact/start"],
            "mcp_tool": ["mcpServerStatus/list", "mcpServer/tool/call"], "interrupt": ["turn/interrupt"],
            "steer": ["turn/steer"]}.get(scenario, [])
class Session:
    def __init__(self, process: object, timeout: int, decision: str = "decline") -> None:
        self.process, self.timeout, self.next_id = process, timeout, 1
        self.events: queue.Queue[dict[str, object]] = queue.Queue(); self.decision = decision
        self.seen: list[dict[str, object]] = []
        self.total = self.stderr_bytes = 0; self.stderr = "none"; self.stderr_truncated = False; self.reader_failure = "none"
        self.thread = threading.Thread(target=self._read, daemon=True)
        self.stderr_thread = threading.Thread(target=self._read_stderr, daemon=True)
        self.thread.start(); self.stderr_thread.start()
    def _read(self) -> None:
        try:
            while raw := self.process.stdout.readline():  # type: ignore[attr-defined]
                self.total += len(raw.encode("utf-8", "replace"))
                if self.total <= LIMIT:
                    try: value = json.loads(raw)
                    except json.JSONDecodeError: continue
                    if isinstance(value, dict): self.events.put(value)
        except (OSError, UnicodeError):
            self.reader_failure = "read-error"
    def _read_stderr(self) -> None:
        for raw in self.process.stderr:  # type: ignore[attr-defined]
            size = len(raw.encode("utf-8", "replace")); self.stderr_truncated |= self.stderr_bytes + size > LIMIT
            self.stderr_bytes = min(LIMIT, self.stderr_bytes + size)
            if self.stderr == "none":
                text = raw.lower()
                self.stderr = next((name for marker, name in (("auth", "authentication"), ("rate limit", "rate-limit"), ("model", "model"), ("config", "configuration"), ("error", "error")) if marker in text), "other")
    def diagnostic(self) -> str:
        status = self.process.poll()  # type: ignore[attr-defined]
        suffix = "+" if self.stderr_truncated else ""
        methods = sorted({str(event.get("method")) for event in self.seen if isinstance(event.get("method"), str)})
        return f"exit={'running' if status is None else status},stdout={self.reader_failure},events={len(self.seen)},methods={methods},queued={self.events.qsize()},stderr={self.stderr},stderrBytes={self.stderr_bytes}{suffix}"
    def send(self, method: str, params: dict[str, object]) -> int:
        request_id, self.next_id = self.next_id, self.next_id + 1
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}) + "\n")  # type: ignore[attr-defined]
        self.process.stdin.flush()  # type: ignore[attr-defined]
        return request_id
    def notify(self, method: str, params: dict[str, object]) -> None:
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": method, "params": params}) + "\n")  # type: ignore[attr-defined]
        self.process.stdin.flush()  # type: ignore[attr-defined]
    def receive(self, timeout: float | None = None) -> dict[str, object]:
        if self.total > LIMIT:
            raise ValueError(f"app-server output exceeded the capture bound ({self.diagnostic()})")
        try:
            return self.events.get(timeout=self.timeout if timeout is None else timeout)
        except queue.Empty as error:
            raise ValueError(f"app-server did not provide the required scenario signal ({self.diagnostic()})") from error
    def response(self, request_id: int) -> dict[str, object]:
        while True:
            event = self.receive()
            if event.get("id") == request_id:
                if "error" in event:
                    raise ValueError("app-server rejected a documented capture request")
                result = event.get("result")
                if isinstance(result, dict):
                    return result
                raise ValueError("app-server response lacked an object result")
            self.observe(event)
    def observe(self, event: dict[str, object]) -> None:
        self.seen.append(event)
        if "id" in event and event.get("method") in {
            "item/commandExecution/requestApproval",
            "item/fileChange/requestApproval",
            "item/tool/requestUserInput",
            "item/permissions/requestApproval",
            "mcpServer/elicitation/request",
        }:
            self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": event["id"], "result": {"decision": self.decision}}) + "\n")  # type: ignore[attr-defined]
            self.process.stdin.flush()  # type: ignore[attr-defined]
    def close(self) -> None:
        if self.process.poll() is None:  # type: ignore[attr-defined]
            pid = getattr(self.process, "pid", None)
            if os.name != "nt" and isinstance(pid, int):
                os.killpg(pid, signal.SIGTERM)
            else:
                self.process.terminate()  # type: ignore[attr-defined]
            wait = getattr(self.process, "wait", None)
            if callable(wait):
                try:
                    wait(timeout=2)
                except subprocess.TimeoutExpired:
                    if os.name != "nt" and isinstance(pid, int):
                        os.killpg(pid, signal.SIGKILL)
                    else:
                        self.process.kill()  # type: ignore[attr-defined]
        self.thread.join(timeout=2)
        self.stderr_thread.join(timeout=2)
def thread_params(scenario: str, model: str) -> dict[str, object]:
    value: dict[str, object] = {"model": model, "ephemeral": True, "approvalPolicy": "untrusted", "sandbox": "read-only", "cwd": str(ROOT)}
    return value
def turn_params(thread_id: str, scenario: str, model: str) -> dict[str, object]:
    value: dict[str, object] = {"threadId": thread_id, "input": [{"type": "text", "text": PROMPTS[scenario]}]}
    if scenario == "plan_mode": value["collaborationMode"] = {"mode": "plan", "settings": {"model": model}}
    return value
def ids(result: dict[str, object], key: str) -> str:
    value = result.get(key)
    if isinstance(value, dict) and isinstance(value.get("id"), str): return value["id"]
    raise ValueError(f"app-server {key} response lacked an id")
def command_started(event: dict[str, object]) -> bool:
    params = event.get("params"); item = params.get("item") if isinstance(params, dict) else None; return event.get("method") == "item/started" and isinstance(item, dict) and item.get("type") == "commandExecution"
def command_completed(event: dict[str, object]) -> bool:
    params = event.get("params"); item = params.get("item") if isinstance(params, dict) else None; return event.get("method") == "item/completed" and isinstance(item, dict) and item.get("type") == "commandExecution"
def plan_mode_applied(event: dict[str, object]) -> bool:
    params = event.get("params"); settings = params.get("threadSettings") if isinstance(params, dict) else None; mode = settings.get("collaborationMode") if isinstance(settings, dict) else None; return event.get("method") == "thread/settings/updated" and isinstance(mode, dict) and mode.get("mode") == "plan"
def compaction_completed(event: dict[str, object]) -> bool:
    params = event.get("params"); item = params.get("item") if isinstance(params, dict) else None; return event.get("method") == "thread/compacted" or event.get("method") == "item/completed" and isinstance(item, dict) and item.get("type") == "contextCompaction"
def capture(binary_path: Path, scenario: str, model: str, mcp_server: str | None = None, configs: list[str] | None = None,
            timeout: int = TIMEOUT, popen: object = subprocess.Popen) -> list[dict[str, object]]:
    configs = configs or []
    process = popen([str(binary_path), *(part for config in configs for part in ("-c", config)), "app-server", "--stdio"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE, text=True, start_new_session=os.name != "nt")
    deadline = time.monotonic() + timeout
    if scenario == "mcp_tool" and popen is subprocess.Popen:
        if not mcp_server: raise ValueError("mcp_tool requires --mcp-server for a reviewed isolated probe")
        return capture_direct(process, mcp_server, deadline)
    session = Session(process, timeout, "acceptForSession" if scenario == "permission_persistent" else "decline")
    try:
        session.response(session.send("initialize", {"clientInfo": {"name": "gent-cli-evidence", "version": "1"}, "capabilities": {"experimentalApi": True}})); session.notify("initialized", {})
        thread_id = ids(session.response(session.send("thread/start", thread_params(scenario, model))), "thread")
        if scenario == "mcp_tool":
            if not mcp_server:
                raise ValueError("mcp_tool requires --mcp-server for a reviewed isolated probe")
            capture_probe(session, thread_id, mcp_server, deadline)
            return session.seen
        turn_id = ids(session.response(session.send("turn/start", turn_params(thread_id, scenario, model))), "turn")
        def wait(predicate: object) -> None:
            while not predicate():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise ValueError(f"app-server did not finish the scenario before its capture deadline ({session.diagnostic()})")
                session.observe(session.receive(min(remaining, timeout)))
        def count(method: str) -> int:
            return sum(item.get("method") == method for item in session.seen)
        def completed(interrupted: bool = False) -> bool:
            return any(item.get("method") == "turn/completed" and (not interrupted or turn_status(item) == "interrupted") for item in session.seen)
        if scenario == "permission_persistent":
            wait(lambda: count("item/commandExecution/requestApproval") == 1 and completed() and sum(command_completed(item) for item in session.seen) >= 2)
            return session.seen
        if scenario == "compaction":
            wait(completed); session.send("thread/compact/start", {"threadId": thread_id})
        if scenario == "interrupt":
            wait(lambda: any(command_started(item) for item in session.seen)); session.send("turn/interrupt", {"threadId": thread_id, "turnId": turn_id})
        if scenario == "steer":
            wait(lambda: any(command_started(item) for item in session.seen)); session.send("turn/steer", {"threadId": thread_id, "expectedTurnId": turn_id,
                                         "input": [{"type": "text", "text": "Stop now. Reply only GENT_STEER_CAPTURE_OK."}]})
        conditions = {"permission_prompt": lambda: count("item/commandExecution/requestApproval") >= 1,
            "permission_persistent": lambda: count("item/commandExecution/requestApproval") == 1 and completed() and sum(command_completed(item) for item in session.seen) >= 2,
            "plan_mode": lambda: any(plan_mode_applied(item) for item in session.seen),
            "compaction": lambda: any(compaction_completed(item) for item in session.seen),
            "interrupt": lambda: completed(True), "steer": lambda: completed()}[scenario]
        wait(conditions)
        return session.seen
    finally:
        session.close()
def required(scenario: str, seen: list[dict[str, object]]) -> set[str]:
    methods = {item.get("method") for item in seen}
    value = {"permission_prompt": {"item/commandExecution/requestApproval"},
            "permission_persistent": {"item/commandExecution/requestApproval", "turn/completed"},
            "plan_mode": {"thread/settings/updated"}, "compaction": {"item/completed"},
            "mcp_tool": {"mcpServer/tool/call"}, "interrupt": {"turn/completed"},
            "steer": {"turn/completed"}}[scenario]
    if scenario == "permission_persistent" and (sum(item.get("method") == "item/commandExecution/requestApproval" for item in seen) != 1 or sum(command_completed(item) for item in seen) < 2): return set()
    if scenario == "interrupt" and not any(item.get("method") == "turn/completed" and turn_status(item) == "interrupted" for item in seen): return set()
    if scenario == "plan_mode" and not any(plan_mode_applied(item) for item in seen) or scenario == "compaction" and not any(compaction_completed(item) for item in seen): return set()
    return value if value.issubset(methods) else set()
def turn_status(event: dict[str, object]) -> object:
    params = event.get("params"); turn = params.get("turn") if isinstance(params, dict) else None
    return turn.get("status") if isinstance(turn, dict) else None
def frames(scenario: str, observed: set[str]) -> list[dict[str, object]]:
    return [{"in": {"nativeType": method}, "expect": method.replace("/", "_"), "expectFields": {"observed": True}}
            for method in sorted(observed)]
def write(path: Path, scenario: str, binary_path: Path, seen: list[dict[str, object]]) -> None:
    observed = required(scenario, seen)
    if not observed:
        raise ValueError("required documented condition was absent; no fixture was written")
    platform_name = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}.get(platform.system(), platform.system().lower())
    normalized = frames(scenario, observed)
    metadata: dict[str, object] = {"vendor": "codex", "scenario": scenario, "status": "recorded", "captureOrigin": "live_cli", "transport": "json_rpc", "adapterSpecVersion": "1", "appVersion": "0.1.4", "prompt": PROMPTS[scenario], "repo": "gent-ar/gent-cli",
        "notes": "Generated from documented Codex app-server JSON-RPC. Raw payloads were bounded and discarded.", "capturedAt": dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "cliVersion": subprocess.run([str(binary_path), "--version"], text=True, capture_output=True, check=True, timeout=15).stdout.strip().removeprefix("codex-cli "),
        "executablePath": str(binary_path), "executableDigest": "sha256:" + hashlib.sha256(binary_path.read_bytes()).hexdigest(),
        "platform": f"{platform_name}-{platform.machine()}", "captureRunId": str(uuid.uuid4()), "attestationScope": "redacted_normalized_fixture_v1"}
    digest = hashlib.sha256(json.dumps({"meta": metadata, "frames": normalized}, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    metadata["attestationDigest"] = "sha256:" + digest
    if path.exists(): raise ValueError("fixture exists; select a new reviewed output path")
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent, delete=False) as file:
        file.write(json.dumps({"meta": metadata}, separators=(",", ":")) + "\n")
        file.writelines(json.dumps(frame, separators=(",", ":")) + "\n" for frame in normalized)
        temporary = Path(file.name)
    os.replace(temporary, path)
def manifest_update(scenario: str, output_name: str, replace: bool) -> tuple[Path, str]:
    manifest = FIXTURES / "manifest.yml"
    text = manifest.read_text(encoding="utf-8")
    pattern = re.compile(rf"\{{\s*vendor:\s*codex,\s*scenario:\s*{scenario},\s*state:\s*(capture_required|recorded)(?:,\s*path:\s*[^}}]+)?\s*}}")
    match = pattern.search(text)
    if match is None or (match.group(1) == "recorded" and not replace):
        raise ValueError(
            "manifest cell is already recorded; pass --replace-existing after reviewing the replacement"
        )
    replacement = f"{{ vendor: codex, scenario: {scenario}, state: recorded, path: {output_name} }}"
    return manifest, text[:match.start()] + replacement + text[match.end():]
def main() -> int:
    value = args(); path = output_path(value.output); summary = plan(value.scenario, value.model, path)
    if not 1 <= value.timeout_seconds <= TIMEOUT: raise ValueError(f"--timeout-seconds must be 1..{TIMEOUT}")
    if value.dry_run:
        print(json.dumps(summary, separators=(",", ":")))
        return 0
    if path.exists() and not value.replace_existing:
        raise ValueError("fixture already exists; pass --replace-existing after reviewing the replacement")
    manifest = manifest_update(value.scenario, path.name, value.replace_existing) if value.update_manifest else None
    if not value.confirm_live_capture: raise ValueError("pass --confirm-live-capture to invoke authenticated Codex")
    path.parent.mkdir(parents=True, exist_ok=True); executable = binary()
    if value.scenario == "mcp_tool" and not value.mcp_server: raise ValueError("mcp_tool requires --mcp-server")
    write(path, value.scenario, executable, capture(executable, value.scenario, value.model, value.mcp_server, value.codex_config, value.timeout_seconds))
    if manifest is not None:
        manifest_path, manifest_text = manifest
        with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=manifest_path.parent, delete=False) as manifest_file:
            manifest_file.write(manifest_text)
            manifest_file_temporary = Path(manifest_file.name)
        os.replace(manifest_file_temporary, manifest_path)
    print(f"wrote redacted codex/{value.scenario}: {path}")
    return 0
if __name__ == "__main__":
    try: raise SystemExit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error: print(f"error: {error}", file=sys.stderr); raise SystemExit(2)
