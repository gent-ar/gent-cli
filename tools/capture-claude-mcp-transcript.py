#!/usr/bin/env python3
"""Capture a bounded, redacted Claude MCP-tool transcript.

The only MCP server is this executable in ``--serve`` mode. It exposes one
argument-free probe and returns a fixed marker; it has no filesystem, network,
credential, or subprocess capability. Raw Claude stream JSON is bounded in
memory solely to correlate the native tool call/result and is never written.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

from public_driver_capture_stream import capture

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "fixtures/public-driver-transcripts"
MAX_CAPTURE_BYTES = 256 * 1024
CAPTURE_TIMEOUT_SECONDS = 90
MODEL_PATTERN = re.compile(r"[A-Za-z0-9._-]+$")
SERVER_NAME = "gent_probe"
TOOL_NAME = "gent_probe_ping"
NATIVE_TOOL_NAME = f"mcp__{SERVER_NAME}__{TOOL_NAME}"
PROMPT = (
    "Use the gent_probe_ping MCP tool exactly once. Do not use any other tool. "
    "After its result, reply exactly GENT_MCP_CAPTURE_OK and nothing else."
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serve", action="store_true", help="run the disposable stdio MCP probe")
    parser.add_argument("--model")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--confirm-live-capture", action="store_true")
    parser.add_argument("--update-manifest", action="store_true")
    parser.add_argument("--replace-existing", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args()


def send(message: dict[str, object]) -> None:
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def serve() -> int:
    """Serve the smallest read-only MCP surface required by the live probe."""
    for line in sys.stdin:
        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue
        request_id, method = request.get("id"), request.get("method")
        if request_id is None:
            continue
        if method == "initialize":
            send({"jsonrpc": "2.0", "id": request_id, "result": {
                "protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
                "serverInfo": {"name": SERVER_NAME, "version": "1"},
            }})
        elif method == "tools/list":
            send({"jsonrpc": "2.0", "id": request_id, "result": {"tools": [{
                "name": TOOL_NAME, "description": "Return a fixed evidence marker.",
                "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
            }]}})
        elif method == "tools/call":
            params = request.get("params")
            valid = isinstance(params, dict) and params.get("name") == TOOL_NAME
            if valid:
                send({"jsonrpc": "2.0", "id": request_id, "result": {
                    "content": [{"type": "text", "text": "GENT_MCP_TOOL_RESULT_OK"}],
                    "isError": False,
                }})
            else:
                send({"jsonrpc": "2.0", "id": request_id, "error": {
                    "code": -32602, "message": "only gent_probe_ping is available",
                }})
        else:
            send({"jsonrpc": "2.0", "id": request_id, "error": {
                "code": -32601, "message": "method unavailable",
            }})
    return 0


def fixture_path(output: Path) -> Path:
    path, root = output.resolve(), FIXTURES.resolve()
    if path.parent != root or path.suffix != ".jsonl" or path.is_symlink():
        raise ValueError("--output must be a non-symlink .jsonl directly in fixtures/public-driver-transcripts")
    return path


def executable() -> Path:
    found = shutil.which("claude")
    if found is None:
        raise ValueError("claude is not on PATH")
    return Path(found).resolve()


def config() -> str:
    return json.dumps({"mcpServers": {SERVER_NAME: {
        "command": sys.executable, "args": [str(Path(__file__).resolve()), "--serve"],
    }}}, separators=(",", ":"))


def command(binary: Path, model: str, mcp_config: str) -> list[str]:
    return [
        str(binary), "--strict-mcp-config", "--mcp-config", mcp_config,
        "--tools", NATIVE_TOOL_NAME, "--allowedTools", NATIVE_TOOL_NAME,
        "--permission-mode", "dontAsk", "--print", "--model", model,
        "--max-budget-usd", "0.05", "--no-session-persistence",
        "--output-format", "stream-json", "--verbose", PROMPT,
    ]


def observed_mcp_tool(raw: str) -> bool:
    """Require the exact configured MCP call, correlated result, and success."""
    call_ids: set[str] = set()
    matched_result = terminal_success = False
    for line in raw.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        message = event.get("message")
        blocks = message.get("content", []) if isinstance(message, dict) else []
        for block in blocks:
            if not isinstance(block, dict):
                continue
            if block.get("type") == "tool_use" and block.get("name") == NATIVE_TOOL_NAME:
                identifier = block.get("id")
                if isinstance(identifier, str) and identifier:
                    call_ids.add(identifier)
            if block.get("type") == "tool_result" and block.get("tool_use_id") in call_ids:
                matched_result = True
        terminal_success |= (event.get("type") == "result" and event.get("subtype") == "success"
                             and event.get("is_error") is False)
    return len(call_ids) == 1 and matched_result and terminal_success


def normalized_frames() -> list[dict[str, object]]:
    return [
        {"in": {"nativeType": NATIVE_TOOL_NAME, "status": "started", "server": SERVER_NAME},
         "expect": "mcp_tool_started", "expectFields": {"tool": TOOL_NAME, "mcp": True}},
        {"in": {"nativeType": "tool_result", "tool": NATIVE_TOOL_NAME, "matchedToolUse": True},
         "expect": "mcp_tool_completed", "expectFields": {"tool": TOOL_NAME, "succeeded": True}},
        {"in": {"nativeType": "result", "subtype": "success"}, "expect": "completed_turn",
         "expectFields": {"terminal": True}},
    ]


def attestation(metadata: dict[str, object], frames: list[dict[str, object]]) -> str:
    reviewed = {"meta": {key: value for key, value in metadata.items() if key != "attestationDigest"}, "frames": frames}
    encoded = json.dumps(reviewed, sort_keys=True, separators=(",", ":")).encode()
    return "sha256:" + hashlib.sha256(encoded).hexdigest()


def provider_version(binary: Path) -> str:
    completed = subprocess.run([str(binary), "--version"], check=False, text=True, capture_output=True, timeout=15)
    value = completed.stdout.strip()
    if completed.returncode or not value or len(value) > 256:
        raise ValueError("could not obtain a bounded provider version; no fixture was written")
    return value


def metadata(binary: Path, model: str, frames: list[dict[str, object]]) -> dict[str, object]:
    captured = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    system = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}.get(platform.system(), platform.system().lower())
    value: dict[str, object] = {
        "vendor": "claude", "scenario": "mcp_tool", "capturedAt": captured,
        "cliVersion": provider_version(binary), "adapterSpecVersion": "1", "appVersion": "0.1.3",
        "prompt": "Bounded local MCP probe; provider response text redacted.", "repo": "gent-ar/gent-cli",
        "notes": "Live Claude call to one disposable local stdio MCP probe was observed. The server exposes one argument-free fixed-marker tool and has no network, credential, filesystem, or subprocess capability. Raw stream JSON and response text were discarded; this attestation covers only reviewed normalized facts.",
        "status": "recorded", "captureOrigin": "live_cli", "executablePath": str(binary),
        "executableDigest": "sha256:" + hashlib.sha256(binary.read_bytes()).hexdigest(),
        "platform": f"{system}-{platform.machine()}", "transport": "stream_json",
        "captureRunId": str(uuid.uuid4()), "attestationScope": "redacted_normalized_fixture_v1",
        "captureModel": model,
    }
    value["attestationDigest"] = attestation(value, frames)
    return value


def atomic_write(path: Path, content: str) -> None:
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent, delete=False) as file:
        file.write(content)
        temporary = Path(file.name)
    os.replace(temporary, path)


def update_manifest(output: Path, replace: bool) -> tuple[Path, str]:
    manifest = FIXTURES / "manifest.yml"
    text = manifest.read_text(encoding="utf-8")
    pattern = r"\{ vendor: claude, scenario: mcp_tool, state: (capture_required|recorded)(?:, path: [^}]+)? \}"
    match = re.search(pattern, text)
    if match is None or (match.group(1) == "recorded" and not replace):
        raise ValueError("MCP manifest cell is already recorded; pass --replace-existing after review")
    replacement = f"{{ vendor: claude, scenario: mcp_tool, state: recorded, path: {output.name} }}"
    return manifest, text[:match.start()] + replacement + text[match.end():]


def main() -> int:
    args = parse_args()
    if args.serve:
        return serve()
    if not args.model or not args.output:
        raise ValueError("--model and --output are required outside --serve mode")
    if not MODEL_PATTERN.fullmatch(args.model):
        raise ValueError("--model may contain only letters, digits, '.', '_', and '-'")
    output = fixture_path(args.output)
    planned = json.dumps({"scenario": "claude/mcp_tool", "output": str(output), "command": command(Path("<claude-resolved-at-live-capture>"), args.model, "<strict-local-mcp-config>"), "rawOutput": "bounded-memory-only"}, separators=(",", ":"))
    if args.dry_run:
        print(planned)
        return 0
    if not args.confirm_live_capture:
        raise ValueError("pass --confirm-live-capture to invoke authenticated Claude")
    if output.exists() and not args.replace_existing:
        raise ValueError("fixture already exists; pass --replace-existing after review")
    manifest = update_manifest(output, args.replace_existing) if args.update_manifest else None
    binary, frames = executable(), normalized_frames()
    raw = capture(command(binary, args.model, config()), MAX_CAPTURE_BYTES, CAPTURE_TIMEOUT_SECONDS)
    if "GENT_MCP_CAPTURE_OK" not in raw or not observed_mcp_tool(raw):
        raise ValueError("native local MCP call/result/success facts were absent; no fixture was written")
    lines = [json.dumps({"meta": metadata(binary, args.model, frames)}, separators=(",", ":"))]
    lines.extend(json.dumps(frame, separators=(",", ":")) for frame in frames)
    atomic_write(output, "\n".join(lines) + "\n")
    if manifest is not None:
        atomic_write(*manifest)
    print(f"wrote redacted claude/mcp_tool: {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
