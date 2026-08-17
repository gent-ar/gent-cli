#!/usr/bin/env python3
"""Capture a bounded, redacted Codex native-subagent transcript after consent."""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import platform
import queue
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "fixtures/public-driver-transcripts"
PROMPT = ("Use exactly one native subagent to answer: what is 2 plus 2? The subagent must not "
          "run tools, read files, or make network requests. Wait for it, then reply only "
          "GENT_CODEX_SUBAGENT_CAPTURE_OK.")
LIMIT, TIMEOUT = 256 * 1024, 90


def args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", default="gpt-5.6-luna")
    parser.add_argument("--output", type=Path, required=True)
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


class Session:
    def __init__(self, process: object, timeout: int) -> None:
        self.process, self.timeout, self.next_id = process, timeout, 1
        self.events: queue.Queue[dict[str, object]] = queue.Queue()
        self.seen: list[dict[str, object]] = []
        self.total = 0
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()

    def read(self) -> None:
        for raw in self.process.stdout:  # type: ignore[attr-defined]
            self.total += len(raw.encode("utf-8", "replace"))
            if self.total > LIMIT:
                continue
            try:
                event = json.loads(raw)
            except json.JSONDecodeError:
                continue
            if isinstance(event, dict):
                self.events.put(event)

    def send(self, method: str, params: dict[str, object]) -> int:
        request_id, self.next_id = self.next_id, self.next_id + 1
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method,
                                             "params": params}) + "\n")  # type: ignore[attr-defined]
        self.process.stdin.flush()  # type: ignore[attr-defined]
        return request_id

    def notify(self, method: str, params: dict[str, object]) -> None:
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": method,
                                             "params": params}) + "\n")  # type: ignore[attr-defined]
        self.process.stdin.flush()  # type: ignore[attr-defined]

    def receive(self, seconds: float) -> dict[str, object]:
        if self.total > LIMIT:
            raise ValueError("app-server output exceeded the capture bound")
        try:
            event = self.events.get(timeout=seconds)
        except queue.Empty as error:
            raise TimeoutError("app-server did not produce the required subagent event") from error
        self.seen.append(event)
        self.decline(event)
        return event

    def response(self, request_id: int, deadline: float) -> dict[str, object]:
        while time.monotonic() < deadline:
            try:
                event = self.receive(min(1, deadline - time.monotonic()))
            except TimeoutError:
                continue
            if event.get("id") == request_id:
                result = event.get("result")
                if isinstance(result, dict):
                    return result
                raise ValueError("app-server rejected a documented capture request")
        raise TimeoutError("app-server request timed out")

    def decline(self, event: dict[str, object]) -> None:
        methods = {"item/commandExecution/requestApproval", "item/fileChange/requestApproval",
                   "item/tool/requestUserInput", "item/permissions/requestApproval"}
        if "id" in event and event.get("method") in methods:
            self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": event["id"],
                                                 "result": {"decision": "decline"}}) + "\n")  # type: ignore[attr-defined]
            self.process.stdin.flush()  # type: ignore[attr-defined]

    def close(self) -> None:
        if self.process.poll() is None:  # type: ignore[attr-defined]
            pid = getattr(self.process, "pid", None)
            if os.name != "nt" and isinstance(pid, int):
                os.killpg(pid, signal.SIGTERM)
            else:
                self.process.terminate()  # type: ignore[attr-defined]
            try:
                self.process.wait(timeout=3)  # type: ignore[attr-defined]
            except subprocess.TimeoutExpired:
                if os.name != "nt" and isinstance(pid, int):
                    os.killpg(pid, signal.SIGKILL)
                else:
                    self.process.kill()  # type: ignore[attr-defined]
        self.reader.join(timeout=2)


def ident(result: dict[str, object], key: str) -> str:
    value = result.get(key)
    if isinstance(value, dict) and isinstance(value.get("id"), str):
        return value["id"]
    raise ValueError(f"app-server {key} response lacked an id")


def item(event: dict[str, object]) -> dict[str, object] | None:
    params = event.get("params")
    value = params.get("item") if isinstance(params, dict) else None
    return value if isinstance(value, dict) else None


def terminal(event: dict[str, object]) -> bool:
    params = event.get("params")
    turn = params.get("turn") if isinstance(params, dict) else None
    return event.get("method") == "turn/completed" and isinstance(turn, dict)


def observed_native_subagent(events: list[dict[str, object]]) -> bool:
    spawned: dict[str, tuple[str, ...]] = {}
    waited: set[str] = set()
    unsafe = {"commandExecution", "fileChange", "webSearch", "dynamicToolCall"}
    for event in events:
        value = item(event)
        if value is None:
            continue
        if value.get("type") in unsafe:
            return False
        if value.get("type") != "collabAgentToolCall":
            continue
        call_id, tool, status = value.get("id"), value.get("tool"), value.get("status")
        receiver_ids = value.get("receiverThreadIds")
        if not isinstance(call_id, str) or not isinstance(receiver_ids, list):
            continue
        receivers = tuple(receiver for receiver in receiver_ids if isinstance(receiver, str))
        if len(receivers) != len(receiver_ids):
            continue
        if tool == "spawnAgent" and status == "completed" and len(receivers) == 1:
            spawned[call_id] = receivers
        if tool == "wait" and status == "completed" and receivers in spawned.values():
            states = value.get("agentsStates")
            if isinstance(states, dict) and all(
                isinstance(states.get(receiver), dict) and states[receiver].get("status") == "completed"
                for receiver in receivers
            ):
                waited.add(call_id)
    return bool(spawned and waited and any(terminal(event) for event in events))


def capture(binary_path: Path, model: str, timeout: int, popen: object = subprocess.Popen) -> list[dict[str, object]]:
    process = popen([str(binary_path), "app-server", "--stdio"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL, text=True, start_new_session=os.name != "nt")
    session, deadline = Session(process, timeout), time.monotonic() + timeout
    try:
        session.response(session.send("initialize", {"clientInfo": {"name": "gent-cli-evidence", "version": "1"},
                                                       "capabilities": {"experimentalApi": True}}), deadline)
        session.notify("initialized", {})
        thread_id = ident(session.response(session.send("thread/start", {"model": model, "ephemeral": True,
            "approvalPolicy": "untrusted", "sandbox": "read-only", "cwd": str(ROOT)}), deadline), "thread")
        session.response(session.send("turn/start", {"threadId": thread_id, "effort": "ultra",
            "approvalPolicy": "untrusted", "input": [{"type": "text", "text": PROMPT}]}), deadline)
        terminal_at: float | None = None
        while time.monotonic() < deadline:
            try:
                event = session.receive(min(1, deadline - time.monotonic()))
            except TimeoutError:
                if terminal_at is not None and time.monotonic() - terminal_at >= 20:
                    break
                continue
            if terminal(event) and terminal_at is None:
                terminal_at = time.monotonic()
            if observed_native_subagent(session.seen):
                return session.seen
        raise ValueError("documented one-subagent lifecycle was absent; no fixture was written")
    finally:
        session.close()


def frames() -> list[dict[str, object]]:
    return [
        {"in": {"nativeType": "collabAgentToolCall", "tool": "spawnAgent", "status": "completed",
                "receiverCount": 1}, "expect": "subagent_spawned", "expectFields": {"subagent": True}},
        {"in": {"nativeType": "collabAgentToolCall", "tool": "wait", "status": "completed",
                "receiverCount": 1, "agentStatus": "completed"}, "expect": "subagent_completed",
         "expectFields": {"subagent": True}},
    ]


def manifest_update(output_name: str, replace: bool) -> tuple[Path, str]:
    path = FIXTURES / "manifest.yml"
    text = path.read_text(encoding="utf-8")
    pattern = re.compile(r"\{ vendor: codex, scenario: subagent, state: (capture_required|recorded)(?:, path: [^}]+)? \}")
    match = pattern.search(text)
    if match is None or (match.group(1) == "recorded" and not replace):
        raise ValueError("subagent manifest cell is already recorded; pass --replace-existing after review")
    return path, pattern.sub(f"{{ vendor: codex, scenario: subagent, state: recorded, path: {output_name} }}", text, count=1)


def write(path: Path, binary_path: Path, events: list[dict[str, object]]) -> None:
    if not observed_native_subagent(events):
        raise ValueError("correlated native subagent lifecycle was absent; no fixture was written")
    system = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}.get(platform.system(), platform.system().lower())
    metadata: dict[str, object] = {"vendor": "codex", "scenario": "subagent", "status": "recorded",
        "captureOrigin": "live_cli", "transport": "json_rpc", "adapterSpecVersion": "1", "appVersion": "0.1.5",
        "prompt": "Bounded native one-subagent probe; provider response text redacted.", "repo": "gent-ar/gent-cli",
        "notes": "Documented app-server effort=ultra emitted correlated spawnAgent and terminal wait facts. Raw payloads were bounded and discarded.",
        "capturedAt": dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "cliVersion": subprocess.run([str(binary_path), "--version"], text=True, capture_output=True, check=True,
                                       timeout=15).stdout.strip().removeprefix("codex-cli "),
        "executablePath": str(binary_path), "executableDigest": "sha256:" + hashlib.sha256(binary_path.read_bytes()).hexdigest(),
        "platform": f"{system}-{platform.machine()}", "captureRunId": str(uuid.uuid4()),
        "attestationScope": "redacted_normalized_fixture_v1"}
    normalized = frames()
    metadata["attestationDigest"] = "sha256:" + hashlib.sha256(json.dumps({"meta": metadata, "frames": normalized},
        sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    if path.exists():
        raise ValueError("fixture exists; select a new reviewed output path")
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent, delete=False) as file:
        file.write(json.dumps({"meta": metadata}, separators=(",", ":")) + "\n")
        file.writelines(json.dumps(frame, separators=(",", ":")) + "\n" for frame in normalized)
        temporary = Path(file.name)
    os.replace(temporary, path)


def main() -> int:
    value = args()
    path = output_path(value.output)
    if not 1 <= value.timeout_seconds <= TIMEOUT:
        raise ValueError(f"--timeout-seconds must be 1..{TIMEOUT}")
    if value.dry_run:
        print(json.dumps({"scenario": "codex/subagent", "method": "turn/start", "effort": "ultra",
                          "rawOutput": f"bounded-memory-only:{LIMIT}"}, separators=(",", ":")))
        return 0
    if not value.confirm_live_capture:
        raise ValueError("pass --confirm-live-capture to invoke authenticated Codex")
    if path.exists() and not value.replace_existing:
        raise ValueError("fixture exists; pass --replace-existing after reviewing it")
    manifest = manifest_update(path.name, value.replace_existing) if value.update_manifest else None
    path.parent.mkdir(parents=True, exist_ok=True)
    executable = binary()
    write(path, executable, capture(executable, value.model, value.timeout_seconds))
    if manifest is not None:
        manifest_path, content = manifest
        with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=manifest_path.parent, delete=False) as file:
            file.write(content)
            temporary = Path(file.name)
        os.replace(temporary, manifest_path)
    print(f"wrote redacted codex/subagent: {path}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, TimeoutError, ValueError, subprocess.SubprocessError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(2)
