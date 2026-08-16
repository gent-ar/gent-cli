#!/usr/bin/env python3
"""Capture a bounded, redacted Claude native-subagent transcript.

The live probe runs only in the caller's disposable working directory. Raw
stream JSON stays in bounded memory and is discarded after native Agent-tool
facts are checked. The committed fixture contains no provider response text.
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
import tempfile
import uuid
from pathlib import Path

from public_driver_capture_stream import capture

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "fixtures/public-driver-transcripts"
MAX_CAPTURE_BYTES = 256 * 1024
CAPTURE_TIMEOUT_SECONDS = 90
MODEL_PATTERN = re.compile(r"[A-Za-z0-9._-]+$")
PROMPT = (
    "Use the Agent tool exactly once with subagent_type gent_probe. Do not answer "
    "until its tool result has returned. Then answer exactly GENT_PARENT_CAPTURE_OK "
    "and nothing else."
)
AGENTS = json.dumps({
    "gent_probe": {
        "description": "A bounded evidence probe. Only return the fixed completion marker.",
        "prompt": "Return exactly GENT_SUBAGENT_CAPTURE_OK and nothing else.",
        "tools": [],
    }
}, separators=(",", ":"))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--confirm-live-capture", action="store_true")
    parser.add_argument("--update-manifest", action="store_true")
    parser.add_argument("--replace-existing", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args()


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


def command(binary: Path, model: str) -> list[str]:
    return [
        str(binary), "--strict-mcp-config", "--agents", AGENTS, "--tools", "Task",
        "--allowedTools", "Task(gent_probe)", "--permission-mode", "dontAsk", "--print",
        "--model", model, "--max-budget-usd", "0.05", "--no-session-persistence",
        "--output-format", "stream-json", "--verbose", "--forward-subagent-text", PROMPT,
    ]


def observed_native_subagent(raw: str) -> bool:
    """Require an exact native Agent call, its matching tool result, and success."""
    agent_ids: set[str] = set()
    matching_result = terminal_success = False
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
            if block.get("type") == "tool_use" and block.get("name") == "Agent":
                identifier = block.get("id")
                if isinstance(identifier, str) and identifier:
                    agent_ids.add(identifier)
            if block.get("type") == "tool_result" and block.get("tool_use_id") in agent_ids:
                matching_result = True
        terminal_success |= (
            event.get("type") == "result" and event.get("subtype") == "success"
            and event.get("is_error") is False
        )
    return bool(agent_ids) and matching_result and terminal_success


def normalized_frames() -> list[dict[str, object]]:
    return [
        {"in": {"nativeType": "Agent", "status": "started", "subagentType": "gent_probe"},
         "expect": "subagent_started", "expectFields": {"provider": "claude", "subagent": True}},
        {"in": {"nativeType": "tool_result", "tool": "Agent", "matchedToolUse": True},
         "expect": "subagent_completed", "expectFields": {"subagent": True}},
        {"in": {"nativeType": "result", "subtype": "success"}, "expect": "completed_turn",
         "expectFields": {"terminal": True}},
    ]


def attestation(metadata: dict[str, object], frames: list[dict[str, object]]) -> str:
    reviewed = {"meta": {key: value for key, value in metadata.items() if key != "attestationDigest"}, "frames": frames}
    return "sha256:" + hashlib.sha256(json.dumps(reviewed, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def provider_version(binary: Path) -> str:
    completed = subprocess.run([str(binary), "--version"], check=False, text=True, capture_output=True, timeout=15)
    value = completed.stdout.strip()
    if completed.returncode or not value or len(value) > 256:
        raise ValueError("could not obtain a bounded provider version; no fixture was written")
    return value


def metadata(binary: Path, model: str, frames: list[dict[str, object]]) -> dict[str, object]:
    now = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    system = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}.get(platform.system(), platform.system().lower())
    value: dict[str, object] = {
        "vendor": "claude", "scenario": "subagent", "capturedAt": now,
        "cliVersion": provider_version(binary), "adapterSpecVersion": "1", "appVersion": "0.1.3",
        "prompt": "Bounded native Agent-tool probe; provider response text redacted.", "repo": "gent-ar/gent-cli",
        "notes": "Live native Agent tool_use, matching tool_result, and successful terminal result were observed. Raw stream JSON and response text were discarded; this attestation covers only reviewed normalized facts.",
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
    pattern = r"\{ vendor: claude, scenario: subagent, state: (capture_required|recorded)(?:, path: [^}]+)? \}"
    match = re.search(pattern, text)
    if match is None or (match.group(1) == "recorded" and not replace):
        raise ValueError("subagent manifest cell is already recorded; pass --replace-existing after review")
    replacement = f"{{ vendor: claude, scenario: subagent, state: recorded, path: {output.name} }}"
    return manifest, text[:match.start()] + replacement + text[match.end():]


def main() -> int:
    args = parse_args()
    if not MODEL_PATTERN.fullmatch(args.model):
        raise ValueError("--model may contain only letters, digits, '.', '_', and '-'")
    output = fixture_path(args.output)
    planned = json.dumps({"scenario": "claude/subagent", "output": str(output), "command": command(Path("<claude-resolved-at-live-capture>"), args.model), "rawOutput": "bounded-memory-only"}, separators=(",", ":"))
    if args.dry_run:
        print(planned)
        return 0
    if not args.confirm_live_capture:
        raise ValueError("pass --confirm-live-capture to invoke authenticated Claude")
    if output.exists() and not args.replace_existing:
        raise ValueError("fixture already exists; pass --replace-existing after review")
    manifest = update_manifest(output, args.replace_existing) if args.update_manifest else None
    binary, frames = executable(), normalized_frames()
    raw = capture(command(binary, args.model), MAX_CAPTURE_BYTES, CAPTURE_TIMEOUT_SECONDS)
    if not observed_native_subagent(raw):
        raise ValueError("native Agent call/result/success facts were absent; no fixture was written")
    lines = [json.dumps({"meta": metadata(binary, args.model, frames)}, separators=(",", ":"))]
    lines.extend(json.dumps(frame, separators=(",", ":")) for frame in frames)
    atomic_write(output, "\n".join(lines) + "\n")
    if manifest is not None:
        atomic_write(*manifest)
    print(f"wrote redacted claude/subagent: {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
