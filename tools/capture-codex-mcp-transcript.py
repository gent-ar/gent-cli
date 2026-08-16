#!/usr/bin/env python3
"""Run the generic Codex capture with one disposable, argument-free MCP probe.

The caller must supply an already-authenticated, isolated ``CODEX_HOME``. This
tool does not read, copy, or inspect credentials and cannot write a fixture
unless the app-server confirms the exact probe call.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CAPTURE = ROOT / "tools/capture-codex-app-server-transcript.py"
SERVER = "gent_probe"
TOOL = "gent_probe"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serve", action="store_true")
    parser.add_argument("--model")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--confirm-live-capture", action="store_true")
    parser.add_argument("--replace-existing", action="store_true")
    parser.add_argument("--update-manifest", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args()


def send(message: dict[str, object]) -> None:
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def serve() -> int:
    """Serve the sole no-side-effect MCP tool used by this evidence capture."""
    for line in sys.stdin:
        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue
        request_id = request.get("id")
        if request_id is None:
            continue
        if request.get("method") == "initialize":
            send({"jsonrpc": "2.0", "id": request_id, "result": {
                "protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
                "serverInfo": {"name": SERVER, "version": "1"},
            }})
        elif request.get("method") == "tools/list":
            send({"jsonrpc": "2.0", "id": request_id, "result": {"tools": [{
                "name": TOOL, "description": "Return a fixed evidence marker.",
                "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
            }]}})
        elif request.get("method") == "tools/call" and request.get("params", {}).get("name") == TOOL:
            send({"jsonrpc": "2.0", "id": request_id, "result": {
                "content": [{"type": "text", "text": "GENT_CODEX_MCP_PROBE_OK"}], "isError": False,
            }})
        else:
            send({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32601, "message": "method unavailable"}})
    return 0


def config_overrides() -> list[str]:
    command = json.dumps(sys.executable)
    arguments = json.dumps([str(Path(__file__).resolve()), "--serve"])
    return [
        "mcp_servers = {}",
        f"mcp_servers.{SERVER}.command = {command}",
        f"mcp_servers.{SERVER}.args = {arguments}",
    ]


def main() -> int:
    args = parse_args()
    if args.serve:
        return serve()
    if not args.model or not args.output:
        raise ValueError("--model and --output are required outside --serve")
    if not os.environ.get("CODEX_HOME"):
        raise ValueError("set CODEX_HOME to a reviewed isolated authenticated Codex home")
    command = [
        sys.executable, str(CAPTURE), "mcp_tool", "--model", args.model, "--output", str(args.output),
        "--mcp-server", SERVER,
    ]
    for override in config_overrides():
        command.extend(["--codex-config", override])
    for name in ("confirm_live_capture", "replace_existing", "update_manifest", "dry_run"):
        if getattr(args, name):
            command.append("--" + name.replace("_", "-"))
    return subprocess.call(command, cwd=ROOT)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(2)
