from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path


SCENARIOS = (
    "full_turn", "tool_use", "tool_error", "thinking", "permission_prompt", "permission_persistent", "plan_mode",
    "subagent", "compaction", "mcp_tool", "resume", "interrupt", "steer", "usage_cost",
)
TRANSPORTS = {"claude": "stream_json", "codex": "json_rpc"}
CELL = re.compile(r"-\s*\{\s*vendor:\s*(\w+),\s*scenario:\s*(\w+),\s*state:\s*(\w+)(?:,\s*path:\s*([^\s}]+))?\s*\}")


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def recorded_cells(corpus: Path) -> dict[tuple[str, str], Path]:
    cells = {}
    for vendor, scenario, state, path in CELL.findall((corpus / "manifest.yml").read_text(encoding="utf-8")):
        if state == "recorded" and path:
            cells[(vendor, scenario)] = corpus / path
    return cells


def missing_evidence(corpus: Path) -> list[str]:
    cells = recorded_cells(corpus)
    return [f"{provider} {scenario}" for provider in TRANSPORTS for scenario in SCENARIOS if (provider, scenario) not in cells]


def fixture_metadata(fixture: Path) -> dict[str, object]:
    return json.loads(fixture.read_text(encoding="utf-8").splitlines()[0])["meta"]


def scenario_proof(provider: str, scenario: str, fixture: Path) -> dict[str, object]:
    meta = fixture_metadata(fixture)
    if (meta.get("vendor"), meta.get("scenario"), meta.get("status")) != (provider, scenario, "recorded"):
        raise ValueError(f"{fixture.name} is not a recorded {provider} {scenario} fixture")
    return {
        "provider_version": meta["cliVersion"].split(" ")[0],
        "platform": meta["platform"],
        "transport": TRANSPORTS[provider],
        "fixture_sha256": sha256(fixture),
        "attestation_sha256": meta["attestationDigest"].removeprefix("sha256:"),
        "capture_run_id": meta["captureRunId"],
    }


def provider_evidence(corpus: Path, provider: str, compatibility_manifest_sha256: str, expires_at: int) -> dict[str, object]:
    missing = missing_evidence(corpus)
    if missing:
        raise ValueError("authority evidence is incomplete; record these cells first: " + ", ".join(missing))
    cells = recorded_cells(corpus)
    inventory = sha256(corpus / "manifest.yml")
    return {
        "schema_version": 1,
        "provider": provider,
        "expires_at_unix_seconds": expires_at,
        "compatibility_manifest_sha256": compatibility_manifest_sha256,
        "transcript_inventory_sha256": inventory,
        "coverage_manifest_sha256": inventory,
        "scenarios": {scenario: scenario_proof(provider, scenario, cells[(provider, scenario)]) for scenario in SCENARIOS},
    }
