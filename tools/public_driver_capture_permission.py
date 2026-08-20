"""Bounded, bidirectional capture of one Claude persistent-permission grant.

Uses Claude Code's own `--permission-prompt-tool stdio` relay — the same mechanism the
production app uses for every session (see `claude_driver.dart`'s `parseControlRequest`/
`encodeControlResponse`) — never an external MCP approval server.

Verified live (2026-08-19): the CLI relays EVERY `can_use_tool` decision through this
channel, every time, even for an identical repeated command and even across a fresh process
that already has the grant written to `.claude/settings.local.json`. Persistence is NOT a
CLI-side behavior — the CLI is a "dumb" per-request relay. It is the CLIENT's job (the
production app, and here this capture tool standing in for it) to remember an already-granted
`updatedPermissions` rule and auto-respond to a later identical request without asking a human
again. "Session-scoped" describes what the CLIENT remembers, not what the CLI enforces.
"""

from __future__ import annotations

import json
import os
import signal
import subprocess
import threading

from public_driver_capture_stream import BoundedReader, _join


def _select_suggestion(suggestions: list) -> dict | None:
    """Prefers a suggestion whose `destination` is `session` over one persisted to disk
    (`localSettings`) — this tool's own in-memory remembered grant is what matters here."""
    for suggestion in suggestions:
        if isinstance(suggestion, dict) and suggestion.get("destination") == "session":
            return suggestion
    return suggestions[0] if suggestions else None


class PersistentPermissionReader(BoundedReader):
    """Grants the first `can_use_tool` request for the expected command, remembers that
    grant, and auto-approves any later identical request without further human input —
    exactly what a persistent-permission-aware client (the production app, or `gentd`) does.
    """

    def __init__(self, limit: int, expected_command: str, stdin: object) -> None:
        super().__init__(limit)
        self.expected_command = expected_command
        self.stdin = stdin
        self.first_approval = threading.Event()
        self.second_approval = threading.Event()
        self.terminal = threading.Event()
        self.first_request: dict | None = None
        self.result: dict | None = None
        self._granted_permission: dict | None = None
        self._approvals = 0

    def drain(self, stream: object) -> None:
        # `readline()`, not a fixed-size `read()`: Claude goes SILENT on stdout while it
        # synchronously waits for our `control_response` on stdin. A `read(n)` that blocks
        # trying to fill a large buffer would never see the pending `control_request` line
        # in time, and Claude aborts the relay ("Stream closed") waiting for a response we
        # never actually got a chance to send.
        for raw_line in iter(stream.readline, b""):
            self.record(raw_line)
            if self.total > self.limit:
                continue
            try:
                event = json.loads(raw_line)
            except (UnicodeDecodeError, json.JSONDecodeError):
                continue
            if event.get("type") == "control_request":
                self._on_control_request(event)
            elif event.get("type") == "result":
                self.result = event
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
        request_id = event.get("request_id") or request.get("request_id")
        if self._approvals == 0:
            self.first_request = event
            suggestions = request.get("permission_suggestions") or request.get("suggestions") or []
            self._granted_permission = _select_suggestion(suggestions)
            self.first_approval.set()
        self._approvals += 1
        response_body: dict[str, object] = {"behavior": "allow", "updatedInput": request.get("input", {})}
        if self._granted_permission is not None:
            response_body["updatedPermissions"] = [self._granted_permission]
        try:
            self.stdin.write((json.dumps({
                "type": "control_response",
                "response": {"subtype": "success", "request_id": request_id, "response": response_body},
            }) + "\n").encode())
            self.stdin.flush()
        except (BrokenPipeError, OSError):
            # The CLI closed its side of the relay (its own internal wait for THIS exact
            # response expired) before our response reached it. Record the miss rather than
            # crash the reader thread; the caller sees no second_approval and fails clearly.
            return
        if self._approvals >= 2:
            self.second_approval.set()


def capture_persistent_permission(probe: list[str], prompt: str, expected_command: str,
                                  limit: int, timeout: int) -> str:
    """Auto-approves the first `can_use_tool` request for the expected command and every
    later identical request, then confirms the CLI never settles the turn as denied.

    Mirrors the app's real control-protocol exchange: the exact `permission_suggestions`
    entry Claude Code itself proposes is echoed back, unmodified, as `updatedPermissions`
    — this tool never fabricates that shape.
    """
    if os.name == "nt":
        raise ValueError("persistent-permission capture requires a POSIX process group; no fixture was written")
    process = subprocess.Popen(probe, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, start_new_session=True)
    assert process.stdin is not None and process.stdout is not None and process.stderr is not None
    stdout = PersistentPermissionReader(limit, expected_command, process.stdin)
    stderr = BoundedReader(limit)
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
        if not stdout.first_approval.wait(timeout):
            raise ValueError("Claude never requested approval for the probe command; no fixture was written")
        if not stdout.second_approval.wait(timeout):
            raise ValueError("the second identical request was never observed; no fixture was written")
        if not stdout.terminal.wait(timeout):
            raise ValueError("the terminal result was absent; no fixture was written")
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
    if stdout.result is None:
        raise ValueError("the terminal result was absent; no fixture was written")
    if stdout.result.get("permission_denials"):
        raise ValueError("Claude reported a permission denial; the auto-grant did not hold")
    if stdout.result.get("is_error"):
        raise ValueError("the provider turn ended in error; no fixture was written")
    return bytes(stdout.data).decode("utf-8", errors="replace")
