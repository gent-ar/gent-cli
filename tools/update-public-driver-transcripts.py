#!/usr/bin/env python3
"""Generate or execute transcript refresh commands for public-driver cells."""

from __future__ import annotations

import argparse
import datetime as dt
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "fixtures/public-driver-transcripts/manifest.yml"
DEFAULT_MODEL = {"claude": "haiku", "codex": "gpt-5.6-luna"}
CAPTURE_PREREQUISITES = {
    ("claude", "permission_persistent"): (
        "requires an installed Claude version that exposes the documented "
        "--permission-prompt-tool interface; then use the bounded local MCP "
        "capture, which records one approval for two identical disposable calls, "
        "never an always-allow permission mode."
    ),
    ("claude", "compaction"): (
        "requires a documented Claude compaction signal or control; do not induce "
        "an unbounded context-overflow capture."
    ),
    ("claude", "malformed_tolerance"): (
        "requires a vendor-documented bounded output-fault control during an "
        "attended read-only/tool-free run; never inject or proxy output. Validate "
        "a reviewed candidate with tools/validate-malformed-driver-evidence.py."
    ),
    ("codex", "subagent"): (
        "requires a documented Codex native-subagent control and correlated event "
        "surface; do not infer a subagent from model text."
    ),
    ("codex", "malformed_tolerance"): (
        "requires a vendor-documented bounded output-fault control during an "
        "attended read-only/tool-free run; never inject or proxy output. Validate "
        "a reviewed candidate with tools/validate-malformed-driver-evidence.py."
    ),
}
CELL_RE = re.compile(
    r"^\s*-\s*\{\s*vendor:\s*(?P<vendor>[a-z]+),\s*"
    r"scenario:\s*(?P<scenario>[a-z_]+),\s*"
    r"state:\s*(?P<state>[a-z_]+)(?:,\s*path:\s*(?P<path>[^ }]+))?\s*\}\s*$"
)


def parse_manifest(path: Path) -> list[dict[str, str]]:
    lines = path.read_text(encoding="utf-8").splitlines()
    cells: list[dict[str, str]] = []
    for line in lines:
        match = CELL_RE.match(line)
        if not match:
            continue
        cell = match.groupdict()
        if cell["path"] is None:
            cell["path"] = ""
        cells.append(cell)
    if not cells:
        raise ValueError("manifest does not expose any parsed cells")
    return cells


def manifest_path(vendor: str, scenario: str, model: str, date: str) -> Path:
    token = model.replace("_", "-").replace(".", "-")
    return ROOT / "fixtures/public-driver-transcripts" / f"{vendor}-{scenario}-{token}-{date}.jsonl"


def command_for(vendor: str, scenario: str, model: str, run_capture: bool = False) -> list[str] | None:
    output = manifest_path(vendor, scenario, model, dt.datetime.now(dt.timezone.utc).strftime("%Y%m%d"))
    if vendor == "claude" and scenario in {
        "full_turn",
        "tool_use",
        "tool_error",
        "thinking",
        "permission_prompt",
        "resume",
        "interrupt",
        "steer",
        "usage_cost",
    }:
        return [
            "python3",
            "tools/capture-public-driver-transcript.py",
            vendor,
            scenario,
            "--model",
            model,
            "--output",
            str(output.relative_to(ROOT)),
            "--confirm-live-capture",
            "--replace-existing",
            "--update-manifest",
        ]
    if vendor == "claude" and scenario == "permission_persistent":
        return [
            "python3", "tools/capture-claude-persistent-permission-transcript.py",
            "--model", model, "--output", str(output.relative_to(ROOT)),
            "--confirm-live-capture", "--replace-existing", "--update-manifest",
        ]
    if vendor == "codex" and scenario in {
        "full_turn",
        "tool_use",
        "tool_error",
        "thinking",
        "resume",
        "usage_cost",
    }:
        return [
            "python3",
            "tools/capture-public-driver-transcript.py",
            vendor,
            scenario,
            "--model",
            model,
            "--output",
            str(output.relative_to(ROOT)),
            "--confirm-live-capture",
            "--replace-existing",
            "--update-manifest",
        ]
    if vendor == "codex" and scenario in {
        "permission_prompt",
        "permission_persistent",
        "plan_mode",
        "compaction",
        "interrupt",
        "steer",
    }:
        dry_run = [] if run_capture else ["--dry-run"]
        return [
            "python3",
            "tools/capture-codex-app-server-transcript.py",
            scenario,
            "--model",
            model,
            "--output",
            str(output.relative_to(ROOT)),
            "--replace-existing",
            "--update-manifest",
            "--confirm-live-capture",
            *dry_run,
        ]
    if vendor == "codex" and scenario == "mcp_tool":
        dry_run = [] if run_capture else ["--dry-run"]
        return [
            "python3",
            "tools/capture-codex-mcp-transcript.py",
            "--model",
            model,
            "--output",
            str(output.relative_to(ROOT)),
            "--replace-existing",
            "--update-manifest",
            "--confirm-live-capture",
            *dry_run,
        ]
    if vendor == "codex" and scenario == "subagent":
        dry_run = [] if run_capture else ["--dry-run"]
        return [
            "python3",
            "tools/capture-codex-subagent-transcript.py",
            "--model",
            model,
            "--output",
            str(output.relative_to(ROOT)),
            "--replace-existing",
            "--update-manifest",
            "--confirm-live-capture",
            *dry_run,
        ]
    return None


def format_command(cmd: list[str]) -> str:
    return " ".join(cmd)


def should_handle_cell(cell: dict[str, str], vendor: str | None, scenario: str | None) -> bool:
    if vendor is not None and cell["vendor"] != vendor:
        return False
    if scenario is not None and cell["scenario"] != scenario:
        return False
    return True


def prerequisite_for(vendor: str, scenario: str) -> str:
    return CAPTURE_PREREQUISITES.get(
        (vendor, scenario),
        "requires a reviewed scenario-specific capture design before any live invocation.",
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vendor", choices=("claude", "codex"))
    parser.add_argument("--scenario")
    parser.add_argument("--model", help="override model for generated command paths")
    parser.add_argument(
        "--run",
        action="store_true",
        help="execute generated refresh commands instead of printing them",
    )
    parser.add_argument(
        "--confirm",
        action="store_true",
        help="required with --run; confirms an attended, reviewed capture session",
    )
    args = parser.parse_args()
    if args.run and not args.confirm:
        raise ValueError("--run requires --confirm")

    if args.scenario and not re.fullmatch(r"[a-z_]+", args.scenario):
        raise ValueError("--scenario must match pattern [a-z_]+")

    model = args.model
    cells = parse_manifest(MANIFEST)
    todo = [cell for cell in cells if cell["state"] != "recorded" and should_handle_cell(cell, args.vendor, args.scenario)]
    if not todo:
        print("No unrecorded cells match this query.")
        return 0

    failures: list[str] = []
    unsupported = [
        cell for cell in todo
        if command_for(cell["vendor"], cell["scenario"], model or DEFAULT_MODEL[cell["vendor"]], args.run) is None
    ]
    if args.run and unsupported:
        print("Capture prerequisites:")
        for cell in unsupported:
            print(f"- {cell['vendor']}:{cell['scenario']}: {prerequisite_for(cell['vendor'], cell['scenario'])}")
        print("No live capture was invoked because this request includes an unsupported cell.")
        return 1
    for cell in todo:
        vendor = cell["vendor"]
        scenario = cell["scenario"]
        resolved_model = model or DEFAULT_MODEL[vendor]
        cmd = command_for(vendor, scenario, resolved_model, args.run)
        if cmd is None:
            failures.append(f"{vendor}:{scenario} (state={cell['state']}): {prerequisite_for(vendor, scenario)}")
            continue
        if args.run:
            print(f"RUN {cell['vendor']} {cell['scenario']} ({cell['state']})")
            exit_code = subprocess.call(cmd, cwd=ROOT)
            if exit_code:
                return exit_code
        else:
            print(format_command(cmd))

    if failures:
        print("Capture prerequisites:")
        for failure in failures:
            print(f"- {failure}")
        return 1
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as exc:
        print(f"update-public-driver-transcripts failed: {exc}", file=sys.stderr)
        sys.exit(1)
