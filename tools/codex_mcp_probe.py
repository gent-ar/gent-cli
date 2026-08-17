"""Bounded, redacted helpers for the isolated Codex MCP evidence probe."""

from __future__ import annotations

import json
import os
import select
import signal
import subprocess
import time
from typing import Protocol

PROBE_MARKER = "GENT_CODEX_MCP_PROBE_OK"


class McpSession(Protocol):
    """The minimal app-server session surface used by the probe."""

    seen: list[dict[str, object]]

    def response(self, request_id: int) -> dict[str, object]: ...

    def send(self, method: str, params: dict[str, object]) -> int: ...


def registered_mcp(result: dict[str, object], server: str) -> bool:
    """Returns whether the exact disposable server exposes the exact probe tool."""
    data = result.get("data")
    return isinstance(data, list) and any(
        isinstance(item, dict)
        and item.get("name") == server
        and isinstance(item.get("tools"), dict)
        and "gent_probe" in item["tools"]
        for item in data
    )


def probe_result(result: dict[str, object]) -> bool:
    """Accepts only the disposable probe's fixed successful response."""
    return result.get("isError") is not True and result.get("content") == [
        {"type": "text", "text": PROBE_MARKER}
    ]


def capture_probe(
    session: McpSession,
    thread_id: str,
    server: str,
    deadline: float,
) -> None:
    """Waits for the isolated server and records one verified structural fact."""
    while True:
        result = session.response(
            session.send(
                "mcpServerStatus/list",
                {"threadId": thread_id, "detail": "full"},
            )
        )
        if registered_mcp(result, server):
            break
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise ValueError("named isolated MCP probe was not ready before the capture deadline")
        time.sleep(min(0.25, remaining))
    result = session.response(
        session.send(
            "mcpServer/tool/call",
            {"threadId": thread_id, "server": server, "tool": "gent_probe", "arguments": {}},
        )
    )
    if not probe_result(result):
        raise ValueError("isolated MCP probe result was absent or invalid")
    session.seen.append(
        {
            "method": "mcpServer/tool/call",
            "params": {"server": server, "tool": "gent_probe", "verified": True},
        }
    )


def capture_direct(process: object, server: str, deadline: float) -> list[dict[str, object]]:
    """Calls the isolated probe without retaining raw live app-server frames."""
    request_id = 1

    def send(method: str, params: dict[str, object]) -> int:
        nonlocal request_id
        current, request_id = request_id, request_id + 1
        process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": current, "method": method, "params": params}) + "\n")  # type: ignore[attr-defined]
        process.stdin.flush()  # type: ignore[attr-defined]
        return current

    def response(expected: int) -> dict[str, object]:
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise ValueError("isolated MCP probe did not respond before the capture deadline")
            ready, _, _ = select.select([process.stdout, process.stderr], [], [], remaining)  # type: ignore[attr-defined]
            if not ready:
                continue
            for stream in ready:
                raw = stream.readline()
                if stream is process.stderr:  # type: ignore[attr-defined]
                    continue
                try:
                    event = json.loads(raw)
                except json.JSONDecodeError:
                    continue
                if not isinstance(event, dict) or event.get("id") != expected:
                    continue
                if "error" in event or not isinstance(event.get("result"), dict):
                    raise ValueError("app-server rejected a documented isolated MCP request")
                return event["result"]

    try:
        response(send("initialize", {"clientInfo": {"name": "gent-cli-evidence", "version": "1"}, "capabilities": {"experimentalApi": True}}))
        process.stdin.write('{"jsonrpc":"2.0","method":"initialized","params":{}}\n')  # type: ignore[attr-defined]
        process.stdin.flush()  # type: ignore[attr-defined]
        thread = response(send("thread/start", {"ephemeral": True, "approvalPolicy": "untrusted", "sandbox": "read-only"}))
        data = thread.get("thread")
        if not isinstance(data, dict) or not isinstance(data.get("id"), str):
            raise ValueError("app-server thread response lacked an id")
        thread_id = data["id"]
        while True:
            status = response(send("mcpServerStatus/list", {"threadId": thread_id, "detail": "full"}))
            if registered_mcp(status, server):
                break
            time.sleep(min(0.25, max(0, deadline - time.monotonic())))
        result = response(send("mcpServer/tool/call", {"threadId": thread_id, "server": server, "tool": "gent_probe", "arguments": {}}))
        if not probe_result(result):
            raise ValueError("isolated MCP probe result was absent or invalid")
        return [{"method": "mcpServer/tool/call", "params": {"server": server, "tool": "gent_probe", "verified": True}}]
    finally:
        if process.poll() is None:  # type: ignore[attr-defined]
            if os.name != "nt": os.killpg(process.pid, signal.SIGTERM)  # type: ignore[attr-defined]
            else: process.terminate()  # type: ignore[attr-defined]
        try: process.wait(timeout=2)  # type: ignore[attr-defined]
        except subprocess.TimeoutExpired:
            if os.name != "nt": os.killpg(process.pid, signal.SIGKILL)  # type: ignore[attr-defined]
            else: process.kill()  # type: ignore[attr-defined]
