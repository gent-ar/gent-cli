"""Minimal `codex app-server` JSON-RPC client for local, on-demand live capture.

gentd's real Codex driver only ever spawns `codex app-server` (see codex_runner.rs,
codex_session/wire.rs) and speaks a small request/response + notification protocol over
stdio: `initialize`, `thread/start`, `turn/start`, then a stream of `method`-keyed
notifications until `turn/completed`/`turn/failed`/`turn/aborted`. This is the one
transport `codex_protocol.rs`'s parser actually understands — `codex exec --json`
(what every other capture tool in this repo drives) emits a structurally different,
unrelated shape and was never a substitute for this.

This client is intentionally narrow: just enough of the protocol to run one turn and
collect its raw notifications for a local drift check. It is not a general app-server
client and makes no attempt at resume, interrupts, MCP config, or approval handling
beyond the fixed policies below.
"""

from __future__ import annotations

import json
import queue
import subprocess
import threading
import time
from pathlib import Path

REQUEST_TIMEOUT_SECONDS = 30
TERMINAL_METHODS = {"turn/completed", "turn/failed", "turn/aborted"}


class AppServerError(ValueError):
    pass


class _LineReader:
    """Reads newline-delimited JSON from a pipe on a background thread into a queue."""

    def __init__(self, stream: object, limit: int) -> None:
        self.queue: queue.Queue[dict | None] = queue.Queue()
        self._stream = stream
        self._limit = limit
        self._total = 0
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def _run(self) -> None:
        try:
            for raw_line in self._stream:
                self._total += len(raw_line)
                if self._total > self._limit:
                    break
                line = raw_line.decode("utf-8", errors="replace").strip()
                if not line:
                    continue
                try:
                    self.queue.put(json.loads(line))
                except json.JSONDecodeError:
                    continue
        finally:
            self.queue.put(None)

    def next(self, deadline: float) -> dict | None:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise AppServerError("codex app-server capture timed out waiting for output")
        try:
            return self.queue.get(timeout=remaining)
        except queue.Empty as error:
            raise AppServerError("codex app-server capture timed out waiting for output") from error


def capture_app_server_turn(
    binary: Path, cwd: str, model: str, effort: str, prompt: str,
    *, limit: int = 256 * 1024, timeout: int = REQUEST_TIMEOUT_SECONDS,
) -> list[str]:
    """Runs one Codex turn over `codex app-server` and returns its raw notification lines.

    Only `method`-keyed notification frames are returned (matching what a real client's
    notification stream looks like) — the request/response envelopes this client itself
    sent and received are not included, the same way production only feeds unsolicited
    notifications through `normalize_public_frame`.
    """
    process = subprocess.Popen(
        [str(binary), "app-server"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, cwd=cwd,
    )
    assert process.stdin is not None and process.stdout is not None
    reader = _LineReader(process.stdout, limit)
    notifications: list[str] = []
    try:
        _request(process, 1, "initialize", {
            "clientInfo": {"name": "gent-drift-check", "version": "0"},
            "capabilities": {"experimentalApi": True, "requestAttestation": False},
        })
        _await_response(reader, 1, timeout)

        _request(process, 2, "thread/start", {"cwd": cwd})
        thread_response = _await_response(reader, 2, timeout)
        thread_id = thread_response.get("result", {}).get("thread", {}).get("id")
        if not isinstance(thread_id, str) or not thread_id:
            raise AppServerError("codex app-server thread/start returned no thread id")

        _request(process, 3, "turn/start", {
            "threadId": thread_id,
            "input": [{"type": "text", "text": prompt}],
            "effort": effort,
            "approvalPolicy": "never",
            "sandboxPolicy": {"type": "readOnly", "networkAccess": False},
            "model": model,
        })

        deadline = time.monotonic() + timeout
        while True:
            frame = reader.next(deadline)
            if frame is None:
                raise AppServerError("codex app-server exited before the turn finished")
            method = frame.get("method")
            if isinstance(method, str):
                notifications.append(json.dumps(frame))
                if method in TERMINAL_METHODS:
                    break
    finally:
        process.stdin.close()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    if not notifications:
        raise AppServerError("codex app-server produced no notifications")
    return notifications


def _request(process: subprocess.Popen, request_id: int, method: str, params: dict) -> None:
    frame = {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
    process.stdin.write((json.dumps(frame) + "\n").encode())
    process.stdin.flush()


def _await_response(reader: _LineReader, request_id: int, timeout: int) -> dict:
    deadline = time.monotonic() + timeout
    while True:
        frame = reader.next(deadline)
        if frame is None:
            raise AppServerError(f"codex app-server exited before request {request_id} answered")
        if frame.get("id") == request_id and "method" not in frame:
            if "error" in frame:
                raise AppServerError(f"codex app-server rejected request {request_id}: {frame['error']}")
            return frame
