#!/usr/bin/env python3
"""Create one bounded, redacted public-driver transcript fixture.

Raw provider output is held briefly in bounded memory, used only to check a
fixed probe marker, then discarded. The committed attestation identifies the
reviewed normalized fixture; it deliberately makes no claim about raw output.
Claurst is private-bridge evidence and is never handled by this public tool.
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

from public_driver_capture_stream import capture, capture_interrupt


ROOT = Path(__file__).resolve().parent.parent
FIXTURE_ROOT = ROOT / "fixtures/public-driver-transcripts"
MAX_CAPTURE_BYTES = 256 * 1024
CAPTURE_TIMEOUT_SECONDS = 90
MODEL_PATTERN = re.compile(r"[A-Za-z0-9._-]+$")
PROBES = {
    "full_turn": "Reply with the exact text GENT_LIVE_CAPTURE_OK and nothing else.",
    "thinking": "Reply with the exact text GENT_THINKING_CAPTURE_OK and nothing else.",
    "tool_use": "Run pwd exactly once. Then reply with the exact text GENT_TOOL_CAPTURE_OK and nothing else.",
    "tool_error": "Run the command false exactly once. Then reply with the exact text GENT_TOOL_ERROR_CAPTURE_OK and nothing else.",
    "permission_prompt": "Run `touch /tmp/gent-public-driver-permission-probe` exactly once. If permission is not granted, reply with exact text GENT_PERMISSION_CAPTURE_OK and nothing else.",
    "interrupt": "Run the command sleep 30 exactly once using the available tool. Do not use any other tool. After it completes, reply exactly GENT_STOP_CAPTURE_OK.",
}
MARKERS = {
    "full_turn": "GENT_LIVE_CAPTURE_OK",
    "thinking": "GENT_THINKING_CAPTURE_OK",
    "tool_use": "GENT_TOOL_CAPTURE_OK",
    "tool_error": "GENT_TOOL_ERROR_CAPTURE_OK",
    "permission_prompt": "GENT_PERMISSION_CAPTURE_OK",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("vendor", choices=("claude", "codex"))
    parser.add_argument("scenario", choices=sorted(PROBES))
    parser.add_argument("--model", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--confirm-live-capture", action="store_true")
    parser.add_argument("--update-manifest", action="store_true")
    parser.add_argument("--replace-existing", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args()


def fixture_path(output: Path) -> Path:
    path = output.resolve()
    root = FIXTURE_ROOT.resolve()
    if path.parent != root or path.suffix != ".jsonl" or path.is_symlink():
        raise ValueError("--output must be a non-symlink .jsonl directly in fixtures/public-driver-transcripts")
    return path


def executable(vendor: str) -> Path:
    found = shutil.which(vendor)
    if found is None:
        raise ValueError(f"{vendor} is not on PATH")
    return Path(found).resolve()


def command(binary: Path, vendor: str, scenario: str, model: str) -> list[str]:
    prompt = PROBES[scenario]
    if vendor == "claude":
        allowed = []
        if scenario == "tool_use":
            allowed = ["--tools", "Bash", "--allowedTools", "Bash(pwd)"]
        if scenario == "tool_error":
            allowed = ["--tools", "Bash", "--allowedTools", "Bash(false)"]
        if scenario == "interrupt":
            allowed = ["--tools", "Bash", "--allowedTools", "Bash(sleep 30)"]
        permission_mode = "manual" if scenario == "permission_prompt" else "dontAsk"
        tools = ["--tools", "Bash"] if scenario == "permission_prompt" else []
        return [str(binary), "--safe-mode", "--strict-mcp-config", *tools, *allowed,
                "--permission-mode", permission_mode, "--print", "--model", model,
                "--max-budget-usd", "0.05", "--no-session-persistence",
                "--output-format", "stream-json", "--verbose", prompt]
    return [str(binary), "exec", "--ephemeral", "--model", model,
            "--sandbox", "read-only", "--json", "--color", "never", prompt]


def dry_run_plan(args: argparse.Namespace) -> str:
    if not MODEL_PATTERN.fullmatch(args.model):
        raise ValueError("--model may contain only letters, digits, '.', '_', and '-'")
    output = fixture_path(args.output)
    planned = command(Path(f"<{args.vendor}-resolved-at-live-capture>"), args.vendor,
                      args.scenario, args.model)
    return json.dumps({"vendor": args.vendor, "scenario": args.scenario, "output": str(output),
                       "command": planned, "rawOutput": "bounded-memory-only",
                       "attestation": "redacted_normalized_fixture_v1"}, separators=(",", ":"))


def version(binary: Path) -> str:
    completed = subprocess.run([str(binary), "--version"], check=False, text=True,
                               capture_output=True, timeout=15)
    value = completed.stdout.strip()
    if completed.returncode or not value or len(value) > 256:
        raise ValueError("could not obtain a bounded provider version; no fixture was written")
    return value.removeprefix("codex-cli ")


def normalized_frames(vendor: str, scenario: str) -> list[dict[str, object]]:
    if scenario == "full_turn":
        return [{"in": {"nativeType": "completed_turn"}, "expect": "completed_turn",
                 "expectFields": {"terminal": True}}]
    if scenario == "thinking":
        if vendor == "codex":
            return [{"in": {"nativeType": "turn.started"}, "expect": "turn_started",
                     "expectFields": {"activity": "thinking"}}]
        return [{"in": {"nativeType": "thinking", "observed": True}, "expect": "thinking",
                 "expectFields": {"activity": "thinking"}}]
    if scenario == "permission_prompt":
        return [
            {"in": {"nativeType": "system.init", "permissionMode": "default", "toolsEnabled": True},
             "expect": "permission_mode_initialized", "expectFields": {"permissionMode": "default"}},
            {"in": {"nativeType": "tool_use", "permissionState": "requested"},
             "expect": "permission_requested", "expectFields": {"decision": "permission"}},
            {"in": {"nativeType": "result", "permissionDenials": 1, "terminalReason": "completed"},
             "expect": "completed_turn", "expectFields": {"terminal": True}},
        ]
    if scenario == "interrupt":
        return [
            {"in": {"nativeType": "Bash", "status": "started"}, "expect": "tool_started",
             "expectFields": {"tool": "Bash"}},
            {"in": {"nativeType": "result", "subtype": "error_during_execution", "isError": True},
             "expect": "interrupted", "expectFields": {"terminal": True, "reason": "interrupted"}},
        ]
    success = scenario == "tool_use"
    tool = "Bash" if vendor == "claude" else "command_execution"
    return [
        {"in": {"nativeType": tool, "status": "started"}, "expect": "tool_started",
         "expectFields": {"tool": tool}},
        {"in": {"nativeType": tool, "status": "completed" if success else "failed",
                "succeeded": success}, "expect": "tool_completed" if success else "tool_failed",
         "expectFields": {"succeeded": success}},
        {"in": {"nativeType": "completed_turn"}, "expect": "completed_turn",
         "expectFields": {"terminal": True}},
    ]


def scenario_was_observed(vendor: str, scenario: str, raw: str) -> bool:
    if scenario == "permission_prompt":
        return vendor == "claude" and claude_permission_was_observed(raw)
    if scenario == "interrupt":
        return vendor == "claude" and claude_interrupt_was_observed(raw)
    if scenario != "thinking":
        return True
    required = '"type":"turn.started"' if vendor == "codex" else '"type":"thinking"'
    return required in raw.replace(" ", "")


def claude_permission_was_observed(raw: str) -> bool:
    """Recognize only the tool request and denial needed for the safe probe."""
    requested = denied = False
    for line in raw.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        message = event.get("message")
        if isinstance(message, dict) and any(
            isinstance(block, dict) and block.get("type") == "tool_use"
            for block in message.get("content", [])
        ):
            requested = True
        if event.get("type") == "result" and event.get("permission_denials"):
            denied = True
    return requested and denied


def claude_interrupt_was_observed(raw: str) -> bool:
    """Require a tool request and Claude's structural cancellation result."""
    requested = interrupted = False
    for line in raw.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        message = event.get("message")
        blocks = message.get("content", []) if isinstance(message, dict) else []
        requested |= any(isinstance(block, dict) and block.get("type") == "tool_use" for block in blocks)
        interrupted |= (event.get("type") == "result"
                        and event.get("subtype") == "error_during_execution"
                        and event.get("is_error") is True)
    return requested and interrupted


def attestation(metadata: dict[str, object], frames: list[dict[str, object]]) -> str:
    reviewed = {"meta": {key: value for key, value in metadata.items() if key != "attestationDigest"},
                "frames": frames}
    encoded = json.dumps(reviewed, sort_keys=True, separators=(",", ":")).encode()
    return "sha256:" + hashlib.sha256(encoded).hexdigest()


def metadata(args: argparse.Namespace, binary: Path, transport: str,
             frames: list[dict[str, object]]) -> dict[str, object]:
    captured = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    platform_name = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}.get(platform.system(), platform.system().lower())
    value: dict[str, object] = {
        "vendor": args.vendor, "scenario": args.scenario, "capturedAt": captured,
        "cliVersion": version(binary), "adapterSpecVersion": "1", "appVersion": "0.1.0",
        "prompt": PROBES[args.scenario], "repo": "gent-ar/gent-cli",
        "notes": "Generated by the bounded redaction-first capture tool. The attestation hashes only reviewed normalized metadata and frames; raw CLI output is discarded and is not attested.",
        "status": "recorded", "captureOrigin": "live_cli", "executablePath": str(binary),
        "executableDigest": "sha256:" + hashlib.sha256(binary.read_bytes()).hexdigest(),
        "platform": f"{platform_name}-{platform.machine()}", "transport": transport,
        "captureRunId": str(uuid.uuid4()), "attestationScope": "redacted_normalized_fixture_v1",
    }
    value["attestationDigest"] = attestation(value, frames)
    return value


def atomic_write(path: Path, content: str) -> None:
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent, delete=False) as file:
        file.write(content)
        temporary = Path(file.name)
    os.replace(temporary, path)


def write_fixture(path: Path, value: dict[str, object], frames: list[dict[str, object]], replace: bool) -> None:
    if path.exists() and not replace:
        raise ValueError("fixture already exists; pass --replace-existing after reviewing the replacement")
    lines = [json.dumps({"meta": value}, separators=(",", ":"))]
    lines.extend(json.dumps(frame, separators=(",", ":")) for frame in frames)
    atomic_write(path, "\n".join(lines) + "\n")


def manifest_update(args: argparse.Namespace, replace: bool) -> tuple[Path, str]:
    manifest = FIXTURE_ROOT / "manifest.yml"
    text = manifest.read_text(encoding="utf-8")
    pattern = rf"{{ vendor: {args.vendor}, scenario: {args.scenario}, state: (capture_required|recorded)(?:, path: [^}}]+)? }}"
    match = re.search(pattern, text)
    if match is None or (match.group(1) == "recorded" and not replace):
        raise ValueError("manifest cell is already recorded; pass --replace-existing after reviewing the replacement")
    replacement = f"{{ vendor: {args.vendor}, scenario: {args.scenario}, state: recorded, path: {args.output.name} }}"
    return manifest, text[:match.start()] + replacement + text[match.end():]


def main() -> int:
    args = parse_args()
    plan = dry_run_plan(args)
    if args.dry_run:
        print(plan)
        return 0
    if not args.confirm_live_capture:
        raise ValueError("pass --confirm-live-capture to invoke an authenticated provider")
    output = fixture_path(args.output)
    if output.exists() and not args.replace_existing:
        raise ValueError("fixture already exists; pass --replace-existing after reviewing the replacement")
    manifest = manifest_update(args, args.replace_existing) if args.update_manifest else None
    binary = executable(args.vendor)
    probe = command(binary, args.vendor, args.scenario, args.model)
    raw = (capture_interrupt(probe, MAX_CAPTURE_BYTES, CAPTURE_TIMEOUT_SECONDS)
           if args.scenario == "interrupt" else capture(probe, MAX_CAPTURE_BYTES, CAPTURE_TIMEOUT_SECONDS))
    if args.scenario != "interrupt" and MARKERS[args.scenario] not in raw:
        raise ValueError("probe marker was absent; no fixture was written")
    if not scenario_was_observed(args.vendor, args.scenario, raw):
        raise ValueError("required normalized scenario signal was absent; no fixture was written")
    frames = normalized_frames(args.vendor, args.scenario)
    write_fixture(output, metadata(args, binary, "stream_json", frames), frames, args.replace_existing)
    if manifest is not None:
        atomic_write(*manifest)
    print(f"wrote redacted {args.vendor}/{args.scenario}: {output}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(2)
