#!/usr/bin/env python3
"""Local-only drift check: do the real, installed Claude/Codex CLIs still parse cleanly?

Captures a real transcript from an authenticated, locally installed `claude`/`codex`
and replays it through the real parser (`gent_drivers::public_protocol::normalize_public_frame`,
with the same tool-result correlation the live Claude runner applies — see
replay-live-driver-transcript.rs), failing loudly if any frame is unrecognized. This is
NOT a CI check: it makes real, billed calls against your own subscription, so it only
ever runs locally, on demand.

The redacted fixtures under fixtures/public-driver-transcripts/ intentionally never
contain real captured provider output (see capture-public-driver-transcript.py), so
they cannot detect a CLI wire-format change. This script is the counterpart that can:
it holds raw output only in memory for the duration of one run and never writes it
to disk or commits anything.

Claude is captured via the one-shot `claude --print --output-format stream-json` path
(public_driver_probes.py — the same command shape capture-public-driver-transcript.py
uses). Codex is captured over `codex app-server`'s JSON-RPC protocol
(public_driver_codex_appserver.py) — the ONLY Codex transport gentd's real driver ever
speaks (codex_runner.rs, CodexSummaryRunner); `codex exec --json`, which every other
tool in this repo drives, emits a structurally unrelated shape codex_protocol.rs was
never built to read, so it is not used here. Codex's `permission_prompt` scenario is
not covered: this client's turn/start always uses `approvalPolicy: never`, so Codex
never emits an approval request to check — handling that would mean answering a live
approval request mid-turn, out of scope for this quick check.

Usage:
    python3 tools/verify-live-driver-parsing.py claude
    python3 tools/verify-live-driver-parsing.py codex
    python3 tools/verify-live-driver-parsing.py claude codex
    python3 tools/verify-live-driver-parsing.py claude --scenario tool_use --scenario thinking
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

from public_driver_capture_stream import capture
from public_driver_codex_appserver import AppServerError, capture_app_server_turn
from public_driver_probes import ONE_SHOT_SCENARIOS, PROBES, command, executable, version

ROOT = Path(__file__).resolve().parent.parent
MAX_CAPTURE_BYTES = 256 * 1024
CAPTURE_TIMEOUT_SECONDS = 90
DEFAULT_MODEL = {"claude": "claude-sonnet-4-5", "codex": "gpt-5.6-terra"}
CODEX_SCENARIOS = tuple(scenario for scenario in ONE_SHOT_SCENARIOS if scenario != "permission_prompt")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("vendor", nargs="+", choices=("claude", "codex"))
    parser.add_argument("--model", help="overrides the default model for every vendor given")
    parser.add_argument(
        "--scenario", action="append", choices=ONE_SHOT_SCENARIOS,
        help="repeatable; defaults to every scenario each vendor supports here "
             "(excludes resume/interrupt/steer always, and permission_prompt for codex; see docstring)",
    )
    return parser.parse_args()


def run_scenario(vendor: str, scenario: str, model: str) -> tuple[bool, str]:
    binary = executable(vendor)
    try:
        if vendor == "claude":
            probe = command(binary, vendor, scenario, model)
            raw = capture(probe, MAX_CAPTURE_BYTES, CAPTURE_TIMEOUT_SECONDS)
            lines = [line for line in raw.splitlines() if line.strip()]
        else:
            lines = capture_app_server_turn(
                binary, str(ROOT), model, "medium", PROBES[scenario],
                limit=MAX_CAPTURE_BYTES, timeout=CAPTURE_TIMEOUT_SECONDS,
            )
    except (ValueError, AppServerError) as error:
        return False, f"capture failed: {error}"
    if not lines:
        return False, "provider produced no output lines"
    replay = subprocess.run(
        ["cargo", "run", "--quiet", "-p", "gent-testkit", "--bin", "replay-live-driver-transcript",
         "--", vendor],
        input="\n".join(lines), text=True, capture_output=True, cwd=ROOT, timeout=120,
    )
    detail = (replay.stdout + replay.stderr).strip()
    return replay.returncode == 0, detail


def main() -> int:
    args = parse_args()
    failures = 0
    for vendor in args.vendor:
        model = args.model or DEFAULT_MODEL[vendor]
        binary = executable(vendor)
        supported = ONE_SHOT_SCENARIOS if vendor == "claude" else CODEX_SCENARIOS
        scenarios = [scenario for scenario in (args.scenario or list(supported)) if scenario in supported]
        print(f"== {vendor} ({version(binary)}) model={model} ==")
        for scenario in scenarios:
            ok, detail = run_scenario(vendor, scenario, model)
            status = "OK  " if ok else "FAIL"
            print(f"  [{status}] {scenario}")
            for line in detail.splitlines():
                print(f"          {line}")
            if not ok:
                failures += 1
    if failures:
        print(f"\n{failures} scenario(s) failed — the parser may need updating for the current CLI.")
        return 1
    print("\nall scenarios parsed cleanly")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(2)
