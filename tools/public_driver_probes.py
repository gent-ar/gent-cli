"""Shared probe scenarios and CLI invocation for public-driver evidence tools.

Single source of truth for what a "scenario" asks a real Claude or Codex CLI to do,
and how its command line is built. Both the committed-fixture capture tool
(capture-public-driver-transcript.py) and the local-only live drift check
(verify-live-driver-parsing.py) import from here so the two never drift apart on
what a scenario means.
"""

from __future__ import annotations

import hashlib
import shutil
import subprocess
from pathlib import Path

PROBES = {
    "full_turn": "Reply with the exact text GENT_LIVE_CAPTURE_OK and nothing else.",
    "thinking": "Reply with the exact text GENT_THINKING_CAPTURE_OK and nothing else.",
    "tool_use": "Run pwd exactly once. Then reply with the exact text GENT_TOOL_CAPTURE_OK and nothing else.",
    "tool_error": "Run the command false exactly once. Then reply with the exact text GENT_TOOL_ERROR_CAPTURE_OK and nothing else.",
    "permission_prompt": "Run `touch /tmp/gent-public-driver-permission-probe` exactly once. If permission is not granted, reply with exact text GENT_PERMISSION_CAPTURE_OK and nothing else.",
    "resume": "Two fixed safe prompts are used: GENT_RESUME_SEED_OK then GENT_RESUME_OK. Neither provider response nor session identity is retained.",
    "interrupt": "Run the command sleep 30 exactly once using the available tool. Do not use any other tool. After it completes, reply exactly GENT_STOP_CAPTURE_OK.",
    "steer": "Run the command sleep 15 exactly once using Bash. Do not use any other tool. Wait for further instructions after it starts.",
}

MARKERS = {
    "full_turn": "GENT_LIVE_CAPTURE_OK",
    "thinking": "GENT_THINKING_CAPTURE_OK",
    "tool_use": "GENT_TOOL_CAPTURE_OK",
    "tool_error": "GENT_TOOL_ERROR_CAPTURE_OK",
    "permission_prompt": "GENT_PERMISSION_CAPTURE_OK",
    "steer": "GENT_STEER_CAPTURE_OK",
}

# Scenarios usable through the one-shot stream-json capture path in this module.
# "resume" (Codex-only JSON-RPC) and "interrupt"/"steer" (Claude-only) are excluded;
# callers that need those reuse public_driver_resume_capture.py / capture_interrupt /
# capture_steer directly, the same way capture-public-driver-transcript.py does.
ONE_SHOT_SCENARIOS = ("full_turn", "thinking", "tool_use", "tool_error", "permission_prompt")


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
        if scenario == "steer":
            allowed = ["--tools", "Bash", "--allowedTools", "Bash(sleep 15)"]
        permission_mode = "manual" if scenario == "permission_prompt" else "dontAsk"
        tools = ["--tools", "Bash"] if scenario == "permission_prompt" else []
        base = [str(binary), "--safe-mode", "--strict-mcp-config", *tools, *allowed,
                "--permission-mode", permission_mode, "--print", "--model", model,
                "--max-budget-usd", "0.05", "--no-session-persistence",
                "--output-format", "stream-json", "--verbose"]
        if scenario == "steer":
            return [*base, "--input-format", "stream-json", "--replay-user-messages"]
        return [*base, prompt]
    return [str(binary), "exec", "--ephemeral", "--model", model,
            "--sandbox", "read-only", "--json", "--color", "never", prompt]


def version(binary: Path) -> str:
    completed = subprocess.run([str(binary), "--version"], check=False, text=True,
                               capture_output=True, timeout=15)
    value = completed.stdout.strip()
    if completed.returncode or not value or len(value) > 256:
        raise ValueError("could not obtain a bounded provider version; no capture was produced")
    return value.removeprefix("codex-cli ")


def executable_digest(binary: Path) -> str:
    return "sha256:" + hashlib.sha256(binary.read_bytes()).hexdigest()
