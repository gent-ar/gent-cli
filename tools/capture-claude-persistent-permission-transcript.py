#!/usr/bin/env python3
"""Capture a bounded, redacted Claude persistent-permission transcript.

Uses Claude Code's own `--permission-prompt-tool stdio` relay — the SAME mechanism the
production app uses on every session (see `claude_driver.dart`'s `parseControlRequest`/
`encodeControlResponse`, and the Claude adapter's spawn manifest) — to grant one Bash
approval as a native `control_response`, then confirm the identical command runs a
second time in the same session with no further prompt. Never spawns an external MCP
approval server: that was this script's original, incorrect design.
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

from public_driver_capture_permission import capture_persistent_permission

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "fixtures/public-driver-transcripts"
MAX_CAPTURE_BYTES = 256 * 1024
CAPTURE_TIMEOUT_SECONDS = 90
MODEL_PATTERN = re.compile(r"[A-Za-z0-9._-]+$")
MARKER = "GENT_PERSISTENT_PERMISSION_OK"


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
    # Mirrors the production app's real spawn (adapter_registry_seed.json's Claude
    # `spawn.args`): `--input-format stream-json --output-format stream-json --print
    # --permission-prompt-tool stdio --verbose`. No `--permission-mode` is passed —
    # matching the app's own default/interactive case — so Claude relays the decision
    # over stdin/stdout instead of auto-denying or auto-accepting it.
    return [str(binary), "--input-format", "stream-json", "--output-format", "stream-json",
            "--print", "--permission-prompt-tool", "stdio", "--verbose",
            "--tools", "Bash", "--model", model, "--max-budget-usd", "0.05",
            "--no-session-persistence"]


def prompt_for(expected: str) -> str:
    return f"Run `{expected}` exactly twice, as two separate tool calls. Do not use any other tool or command. Then reply exactly {MARKER} and nothing else."


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
        "notes": "One native control_response (behavior:allow, updatedPermissions echoing Claude's own permission_suggestions entry) authorized two identical safe Bash calls in one live Claude session, over the same --permission-prompt-tool stdio relay the production app uses. Raw stream JSON, approval input, provider text, and temporary path were discarded.", "status": "recorded", "captureOrigin": "live_cli", "executablePath": str(binary), "executableDigest": "sha256:" + hashlib.sha256(binary.read_bytes()).hexdigest(), "platform": f"{platform_name}-{platform.machine()}", "transport": "stream_json_bidirectional", "captureRunId": str(uuid.uuid4()), "attestationScope": "redacted_normalized_fixture_v1", "captureModel": model}
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
    if not MODEL_PATTERN.fullmatch(args.model): raise ValueError("--model may contain only letters, digits, '.', '_', and '-'")
    output = fixture_path(args.output)
    plan = json.dumps({"scenario": "claude/permission_persistent", "output": str(output), "approval": "exact disposable command only", "rawOutput": "bounded-memory-only"}, separators=(",", ":"))
    if args.dry_run: print(plan); return 0
    if not args.confirm_live_capture: raise ValueError("pass --confirm-live-capture to invoke authenticated Claude")
    if output.exists() and not args.replace_existing: raise ValueError("fixture already exists; pass --replace-existing after review")
    manifest = update_manifest(output, args.replace_existing) if args.update_manifest else None
    binary = executable()
    with tempfile.TemporaryDirectory(prefix="gent-claude-permission-") as directory:
        expected = f"mkdir -p {Path(directory) / 'approved'}"
        items = frames()
        capture_persistent_permission(
            command(binary, args.model), prompt_for(expected), expected,
            MAX_CAPTURE_BYTES, CAPTURE_TIMEOUT_SECONDS,
        )
    lines = [json.dumps({"meta": metadata(binary, args.model, items)}, separators=(",", ":"))] + [json.dumps(item, separators=(",", ":")) for item in items]
    atomic_write(output, "\n".join(lines) + "\n")
    if manifest is not None: atomic_write(*manifest)
    print(f"wrote redacted claude/permission_persistent: {output}")
    return 0


if __name__ == "__main__":
    try: raise SystemExit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"error: {error}", file=sys.stderr); raise SystemExit(2)
