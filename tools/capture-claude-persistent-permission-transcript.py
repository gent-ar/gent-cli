#!/usr/bin/env python3
"""Capture a bounded, redacted Claude session-permission transcript.

The temporary MCP approval server permits one exact mkdir command only. It has
no network, credential, filesystem, or subprocess access beyond a transient
integer counter written by this parent process. A fixture is written only when
one approval authorizes two matching Bash calls in the same live session.
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
SERVER = "gent_permission_probe"
TOOL = "approval_prompt"
NATIVE_TOOL = f"mcp__{SERVER}__{TOOL}"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serve", action="store_true")
    parser.add_argument("--expected-command")
    parser.add_argument("--counter", type=Path)
    parser.add_argument("--model")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--confirm-live-capture", action="store_true")
    parser.add_argument("--update-manifest", action="store_true")
    parser.add_argument("--replace-existing", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args()


def send(value: dict[str, object]) -> None:
    sys.stdout.write(json.dumps(value, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def permitted(arguments: object, expected: str) -> bool:
    if not isinstance(arguments, dict) or arguments.get("tool_name") != "Bash":
        return False
    input_value = arguments.get("input")
    return isinstance(input_value, dict) and input_value.get("command") == expected


def serve(expected: str, counter: Path) -> int:
    approvals = 0
    for line in sys.stdin:
        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue
        identifier, method = request.get("id"), request.get("method")
        if identifier is None:
            continue
        if method == "initialize":
            send({"jsonrpc": "2.0", "id": identifier, "result": {
                "protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
                "serverInfo": {"name": SERVER, "version": "1"},
            }})
        elif method == "tools/list":
            send({"jsonrpc": "2.0", "id": identifier, "result": {"tools": [{
                "name": TOOL, "description": "Approve one exact disposable Bash command.",
                "inputSchema": {"type": "object", "additionalProperties": True},
            }]}})
        elif method == "tools/call":
            params = request.get("params", {})
            allowed = isinstance(params, dict) and params.get("name") == TOOL and permitted(params.get("arguments"), expected)
            if allowed:
                approvals += 1
                counter.write_text(str(approvals), encoding="utf-8")
                payload = {"behavior": "allow", "updatedInput": params["arguments"]["input"]}
            else:
                payload = {"behavior": "deny", "message": "only the disposable evidence command is permitted"}
            send({"jsonrpc": "2.0", "id": identifier, "result": {
                "content": [{"type": "text", "text": json.dumps(payload, separators=(",", ":"))}],
                "isError": False,
            }})
        else:
            send({"jsonrpc": "2.0", "id": identifier, "error": {"code": -32601, "message": "method unavailable"}})
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


def config(command: str, counter: Path) -> str:
    return json.dumps({"mcpServers": {SERVER: {"command": sys.executable, "args": [
        str(Path(__file__).resolve()), "--serve", "--expected-command", command, "--counter", str(counter),
    ]}}}, separators=(",", ":"))


def command(binary: Path, model: str, mcp_config: str, expected: str) -> list[str]:
    prompt = f"Run `{expected}` exactly twice. Do not use any other tool or command. Then reply exactly GENT_PERSISTENT_PERMISSION_OK and nothing else."
    return [str(binary), "--strict-mcp-config", "--mcp-config", mcp_config, "--tools", "Bash",
            "--permission-prompt-tool", NATIVE_TOOL, "--permission-mode", "manual", "--print",
            "--model", model, "--max-budget-usd", "0.05", "--no-session-persistence",
            "--output-format", "stream-json", "--verbose", prompt]


def observed(raw: str, expected: str) -> bool:
    calls: set[str] = set()
    results: set[str] = set()
    success = False
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
            input_value = block.get("input")
            if (block.get("type") == "tool_use" and block.get("name") == "Bash"
                    and isinstance(input_value, dict) and input_value.get("command") == expected):
                identifier = block.get("id")
                if isinstance(identifier, str): calls.add(identifier)
            if block.get("type") == "tool_result" and isinstance(block.get("tool_use_id"), str):
                results.add(block["tool_use_id"])
        success |= event.get("type") == "result" and event.get("subtype") == "success" and event.get("is_error") is False
    return len(calls) == 2 and calls <= results and success and "GENT_PERSISTENT_PERMISSION_OK" in raw


def frames() -> list[dict[str, object]]:
    return [
        {"in": {"nativeType": "permission_prompt", "approvalCount": 1}, "expect": "permission_granted", "expectFields": {"sessionScoped": True}},
        {"in": {"nativeType": "Bash", "status": "completed", "matchedApproval": True}, "expect": "tool_completed", "expectFields": {"succeeded": True, "count": 2}},
        {"in": {"nativeType": "result", "subtype": "success"}, "expect": "completed_turn", "expectFields": {"terminal": True}},
    ]


def metadata(binary: Path, model: str, items: list[dict[str, object]]) -> dict[str, object]:
    captured = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    platform_name = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}.get(platform.system(), platform.system().lower())
    value: dict[str, object] = {"vendor": "claude", "scenario": "permission_persistent", "capturedAt": captured,
        "cliVersion": subprocess.run([str(binary), "--version"], text=True, capture_output=True, check=True, timeout=15).stdout.strip(),
        "adapterSpecVersion": "1", "appVersion": "0.1.5", "prompt": "Bounded disposable two-call permission probe; provider text and temporary path redacted.", "repo": "gent-ar/gent-cli",
        "notes": "One documented noninteractive MCP permission decision approved two identical safe Bash calls in one live Claude session. Raw stream JSON, approval input, provider text, and temporary path were discarded.", "status": "recorded", "captureOrigin": "live_cli", "executablePath": str(binary), "executableDigest": "sha256:" + hashlib.sha256(binary.read_bytes()).hexdigest(), "platform": f"{platform_name}-{platform.machine()}", "transport": "stream_json", "captureRunId": str(uuid.uuid4()), "attestationScope": "redacted_normalized_fixture_v1", "captureModel": model}
    reviewed = {"meta": value, "frames": items}
    value["attestationDigest"] = "sha256:" + hashlib.sha256(json.dumps(reviewed, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    return value


def atomic_write(path: Path, content: str) -> None:
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent, delete=False) as file:
        file.write(content)
        temporary = Path(file.name)
    os.replace(temporary, path)


def update_manifest(output: Path, replace: bool) -> tuple[Path, str]:
    manifest = FIXTURES / "manifest.yml"
    text = manifest.read_text(encoding="utf-8")
    pattern = r"\{ vendor: claude, scenario: permission_persistent, state: (capture_required|recorded)(?:, path: [^}]+)? \}"
    match = re.search(pattern, text)
    if match is None or (match.group(1) == "recorded" and not replace):
        raise ValueError("persistent-permission manifest cell is already recorded; pass --replace-existing after review")
    replacement = f"{{ vendor: claude, scenario: permission_persistent, state: recorded, path: {output.name} }}"
    return manifest, text[:match.start()] + replacement + text[match.end():]


def main() -> int:
    args = parse_args()
    if args.serve:
        if not args.expected_command or not args.counter: raise ValueError("--serve needs --expected-command and --counter")
        return serve(args.expected_command, args.counter)
    if not args.model or not args.output: raise ValueError("--model and --output are required outside --serve")
    if not MODEL_PATTERN.fullmatch(args.model): raise ValueError("--model may contain only letters, digits, '.', '_', and '-'")
    output = fixture_path(args.output)
    plan = json.dumps({"scenario": "claude/permission_persistent", "output": str(output), "approval": "exact disposable command only", "rawOutput": "bounded-memory-only"}, separators=(",", ":"))
    if args.dry_run: print(plan); return 0
    if not args.confirm_live_capture: raise ValueError("pass --confirm-live-capture to invoke authenticated Claude")
    if output.exists() and not args.replace_existing: raise ValueError("fixture already exists; pass --replace-existing after review")
    manifest = update_manifest(output, args.replace_existing) if args.update_manifest else None
    with tempfile.TemporaryDirectory(prefix="gent-claude-permission-") as directory:
        expected = f"mkdir -p {Path(directory) / 'approved'}"
        counter = Path(directory) / "approval-count"
        binary, items = executable(), frames()
        raw = capture(command(binary, args.model, config(expected, counter), expected), MAX_CAPTURE_BYTES, CAPTURE_TIMEOUT_SECONDS)
        approvals = counter.read_text(encoding="utf-8") if counter.exists() else "0"
        if approvals != "1" or not observed(raw, expected): raise ValueError("one approval plus two matching executions was absent; no fixture was written")
    lines = [json.dumps({"meta": metadata(binary, args.model, items)}, separators=(",", ":"))] + [json.dumps(item, separators=(",", ":")) for item in items]
    atomic_write(output, "\n".join(lines) + "\n")
    if manifest is not None: atomic_write(*manifest)
    print(f"wrote redacted claude/permission_persistent: {output}")
    return 0


if __name__ == "__main__":
    try: raise SystemExit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"error: {error}", file=sys.stderr); raise SystemExit(2)
