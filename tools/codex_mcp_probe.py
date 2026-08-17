"""Bounded, redacted helpers for the isolated Codex MCP evidence probe."""

from __future__ import annotations

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
