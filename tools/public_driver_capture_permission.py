"""Bounded, bidirectional capture of one Claude persistent-permission grant.

Uses Claude Code's own `--permission-prompt-tool stdio` relay — the same mechanism the
production app uses for every session (see `claude_driver.dart`'s `parseControlRequest`/
`encodeControlResponse`) — never an external MCP approval server.
"""

from __future__ import annotations

import json
import os
import signal
import subprocess
import threading

from public_driver_capture_stream import BoundedReader, _join


class PersistentPermissionReader(BoundedReader):
    """Recognizes one `can_use_tool` approval, its persistence, and the terminal result.

    Persistence is confirmed by the ABSENCE of a second `control_request` for the
    identical command after the first is granted, alongside a second real `tool_use` for
    that command — the same two facts Claude Code's own session permission store proves.
    """

    def __init__(self, limit: int, expected_command: str) -> None:
        super().__init__(limit)
        self.expected_command = expected_command
        self.pending = bytearray()
        self.approval_requested = threading.Event()
        self.reprompted = threading.Event()
        self.second_call_seen = threading.Event()
        self.terminal = threading.Event()
        self.request: dict | None = None
        self._first_request_id: object | None = None
        self._command_calls = 0

    def drain(self, stream: object) -> None:
        while chunk := stream.read(8192):
            self.record(chunk)
            if self.total <= self.limit:
                self.pending.extend(chunk)
                self.consume_lines()

    def consume_lines(self) -> None:
        while b"\n" in self.pending:
            line, _, remainder = self.pending.partition(b"\n")
            self.pending = bytearray(remainder)
            try:
                event = json.loads(line)
            except (UnicodeDecodeError, json.JSONDecodeError):
                continue
            if event.get("type") == "control_request":
                self._on_control_request(event)
            elif event.get("type") == "assistant":
                self._on_assistant(event)
            if event.get("type") == "result":
                self.terminal.set()

    def _on_control_request(self, event: dict) -> None:
        request = event.get("request")
        if not isinstance(request, dict) or request.get("subtype") != "can_use_tool":
            return
        if request.get("tool_name") != "Bash":
            return
        input_value = request.get("input")
        command = input_value.get("command") if isinstance(input_value, dict) else None
        if command != self.expected_command:
            return
        if self._first_request_id is None:
            self._first_request_id = event.get("request_id") or request.get("request_id")
            self.request = event
            self.approval_requested.set()
        else:
            # A second control_request for the identical command means the granted
            # permission did not persist for the rest of this session.
            self.reprompted.set()

    def _on_assistant(self, event: dict) -> None:
        message = event.get("message")
        blocks = message.get("content", []) if isinstance(message, dict) else []
        for block in blocks:
            if not isinstance(block, dict) or block.get("type") != "tool_use":
                continue
            if block.get("name") != "Bash":
                continue
            input_value = block.get("input")
            if isinstance(input_value, dict) and input_value.get("command") == self.expected_command:
                self._command_calls += 1
                if self._command_calls >= 2:
                    self.second_call_seen.set()


def capture_persistent_permission(probe: list[str], prompt: str, expected_command: str,
                                  limit: int, timeout: int) -> str:
    """Grant one Bash approval via a native `control_response`, then confirm the identical
    command runs a second time with no further `can_use_tool` prompt.

    Mirrors the app's real control-protocol exchange: the exact `permission_suggestions`
    entry Claude Code itself proposes is echoed back, unmodified, as `updatedPermissions`
    — this tool never fabricates that shape.
    """
    if os.name == "nt":
        raise ValueError("persistent-permission capture requires a POSIX process group; no fixture was written")
    process = subprocess.Popen(probe, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, start_new_session=True)
    assert process.stdin is not None and process.stdout is not None and process.stderr is not None
    stdout, stderr = PersistentPermissionReader(limit, expected_command), BoundedReader(limit)
    readers = [threading.Thread(target=reader.drain, args=(stream,)) for reader, stream in
               ((stdout, process.stdout), (stderr, process.stderr))]
    for reader in readers:
        reader.start()
    try:
        process.stdin.write((json.dumps({
            "type": "user",
            "message": {"role": "user", "content": [{"type": "text", "text": prompt}]},
            "parent_tool_use_id": None,
        }) + "\n").encode())
        process.stdin.flush()
        if not stdout.approval_requested.wait(timeout):
            raise ValueError("Claude never requested approval for the probe command; no fixture was written")
        request = stdout.request
        assert request is not None
        inner = request["request"]
        request_id = request.get("request_id") or inner.get("request_id")
        suggestions = inner.get("permission_suggestions") or inner.get("suggestions") or []
        response_body: dict[str, object] = {"behavior": "allow", "updatedInput": inner.get("input", {})}
        if suggestions:
            response_body["updatedPermissions"] = [suggestions[0]]
        process.stdin.write((json.dumps({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": request_id, "response": response_body},
        }) + "\n").encode())
        process.stdin.close()
        status = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired as error:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        _join(readers)
        raise ValueError("persistent-permission capture timed out; no fixture was written") from error
    except (OSError, ValueError):
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        _join(readers)
        raise
    _join(readers)
    if stdout.total > limit or stderr.total > limit or status:
        raise ValueError("persistent-permission provider output was invalid; no fixture was written")
    if stdout.reprompted.is_set():
        raise ValueError("Claude re-requested approval for the identical command; permission did not persist")
    if not stdout.second_call_seen.is_set() or not stdout.terminal.is_set():
        raise ValueError("the second disposable command or terminal result was absent; no fixture was written")
    return bytes(stdout.data).decode("utf-8", errors="replace")
