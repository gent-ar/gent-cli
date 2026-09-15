#!/usr/bin/env python3
from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import platform
import re
import select
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from provider_contract_diff import diff_snapshots
from provider_contract_help import parse_help
from provider_contract_schema import claurst_protocol, codex_protocol


ROOT = Path(__file__).resolve().parents[1]
CONTRACTS = ROOT / "fixtures" / "provider-contracts"
TOOL = "provider-contract-snapshot/1"
PROBE_SECONDS = 25
HELP_PATHS = {
    "claude": ((), ("auth",), ("auth", "status"), ("auth", "login")),
    "codex": ((), ("app-server",), ("login",), ("login", "status")),
}
CLAUDE_STREAM = ["--input-format", "stream-json", "--output-format", "stream-json", "--print", "--verbose", "--include-partial-messages", "--replay-user-messages"]
CLAUDE_VARIANTS = {
    "chat": ["--model", "haiku", "--effort", "medium", "--permission-prompt-tool", "stdio", "--permission-mode", "manual"],
    "chatPlan": ["--model", "haiku", "--effort", "max", "--permission-prompt-tool", "stdio", "--permission-mode", "plan", "--append-system-prompt", "probe", "--max-turns", "3", "--disallowedTools", "WebFetch"],
    "chatSystemPrompt": ["--model", "haiku", "--effort", "low", "--permission-prompt-tool", "stdio", "--permission-mode", "bypassPermissions", "--system-prompt", "probe", "--mcp-config", "{mcp}"],
    "summary": ["--model", "haiku", "--safe-mode", "--permission-mode", "dontAsk", "--tools", "", "--max-turns", "1", "--no-session-persistence"],
}
CLAUDE_INITIALIZE = {"type": "control_request", "request_id": "gent-contract-snapshot", "request": {"subtype": "initialize"}}
CLAUDE_SUBTYPE_LITERAL = re.compile(rb'subtype(?::|===?)\s*(?:[A-Za-z_$][A-Za-z0-9_$]{0,2}\()?"([a-z][a-z0-9_]{2,48})"')
CODEX_APP_SERVER = ["app-server", "-c", "check_for_update_on_startup=false"]
PACKAGE_MANAGER_PROVENANCE = ("CODEX_MANAGED_BY_NPM", "CODEX_MANAGED_BY_BUN", "CODEX_MANAGED_BY_PNPM", "CODEX_MANAGED_BY_VITE_PLUS", "CODEX_MANAGED_PACKAGE_ROOT")
CODEX_INITIALIZE = {"id": 1, "method": "initialize", "params": {"clientInfo": {"name": "gent", "version": "contract-snapshot"}, "capabilities": {"experimentalApi": True, "requestAttestation": False}}}


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_text(command: list[str], environment: dict[str, str] | None = None) -> str:
    result = subprocess.run(command, capture_output=True, text=True, timeout=60, env=environment, stdin=subprocess.DEVNULL)
    if result.returncode != 0:
        raise ValueError(f"{' '.join(command)} exited {result.returncode}: {result.stderr.strip()[:300]}")
    return result.stdout


def exchange(command: list[str], frame: dict[str, object], matches, cwd: Path, environment: dict[str, str]) -> tuple[dict[str, object] | None, str]:
    process = subprocess.Popen(command, cwd=cwd, env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    buffer, response, deadline = b"", None, time.monotonic() + PROBE_SECONDS
    try:
        process.stdin.write(json.dumps(frame).encode() + b"\n")
        process.stdin.flush()
        while response is None and time.monotonic() < deadline:
            ready, _, _ = select.select([process.stdout], [], [], 0.5)
            if not ready:
                if process.poll() is not None:
                    break
                continue
            chunk = os.read(process.stdout.fileno(), 65536)
            if not chunk:
                break
            buffer += chunk
            while b"\n" in buffer and response is None:
                line, buffer = buffer.split(b"\n", 1)
                value = json.loads(line) if line.strip().startswith(b"{") else None
                if isinstance(value, dict) and matches(value):
                    response = value
    finally:
        process.kill()
        _, stderr = process.communicate(timeout=10)
    return response, stderr.decode("utf-8", "replace").strip()


def key_tree(value: object, path: str = "") -> dict[str, str]:
    if isinstance(value, dict):
        tree = {path or "/": "object"}
        for key, child in value.items():
            tree.update(key_tree(child, f"{path}/{key}"))
        return tree
    if isinstance(value, list):
        tree = {path or "/": "array"}
        for child in value[:1]:
            tree.update(key_tree(child, f"{path}/[]"))
        return tree
    return {path or "/": type(value).__name__}


def version_of(provider: str, output: str) -> str:
    match = re.search(r"\d+\.\d+\.\d+", output)
    if match is None:
        raise ValueError(f"{provider} --version did not report a semantic version: {output!r}")
    return match.group(0)


def cli_surface(provider: str, executable: Path) -> dict[str, object]:
    return {" ".join(path) or "<root>": parse_help(run_text([str(executable), *path, "--help"])) for path in HELP_PATHS[provider]}


def claude_capture(executable: Path, scratch: Path) -> dict[str, object]:
    empty_config = scratch / "claude-config"
    empty_config.mkdir()
    mcp = scratch / "mcp.json"
    mcp.write_text(json.dumps({"mcpServers": {}}), encoding="utf-8")
    environment = {**os.environ, "CLAUDE_CONFIG_DIR": str(empty_config)}
    matches = lambda value: value.get("type") == "control_response"
    probes, initialize = {}, None
    for name, variant in CLAUDE_VARIANTS.items():
        argv = [str(executable), *CLAUDE_STREAM, *[str(mcp) if item == "{mcp}" else item for item in variant]]
        response, stderr = exchange(argv, CLAUDE_INITIALIZE, matches, scratch, environment)
        subtype = response.get("response", {}).get("subtype") if response else None
        probes[name] = {"argv": [item if item != str(mcp) else "<mcp-config>" for item in argv[1:]], "handshake": subtype or "missing", "stderr": stderr}
        if subtype == "success" and initialize is None:
            initialize = response["response"]["response"]
    if initialize is None:
        raise ValueError("claude did not answer initialize on any driver argv")
    commands = {entry["name"]: {"argumentHint": entry.get("argumentHint") or "", "aliases": sorted(entry.get("aliases", []))} for entry in initialize.get("commands", [])}
    models = {entry["value"]: {"efforts": sorted(entry.get("supportedEffortLevels", [])), "keys": sorted(entry)} for entry in initialize.get("models", [])}
    protocol = {"initialize": key_tree({key: value for key, value in initialize.items() if key not in ("commands", "models", "agents", "account")})}
    frames = {"subtypes": sorted({match.decode() for match in CLAUDE_SUBTYPE_LITERAL.findall(executable.read_bytes())})}
    return {"protocol.json": protocol, "commands.json": dict(sorted(commands.items())), "models.json": dict(sorted(models.items())), "frames.json": frames, "launch-probe.json": probes}


def codex_capture(executable: Path, scratch: Path) -> dict[str, object]:
    schema = scratch / "schema"
    run_text([str(executable), "app-server", "generate-json-schema", "--out", str(schema)])
    document = json.loads((schema / "codex_app_server_protocol.schemas.json").read_text(encoding="utf-8"))
    environment = {key: value for key, value in os.environ.items() if key not in PACKAGE_MANAGER_PROVENANCE}
    response, stderr = exchange([str(executable), *CODEX_APP_SERVER], CODEX_INITIALIZE, lambda value: value.get("id") == 1, scratch, environment)
    handshake = "success" if response and "result" in response else "error" if response else "missing"
    probe = {"appServer": {"argv": CODEX_APP_SERVER, "handshake": handshake, "stderr": stderr, "result": key_tree(response.get("result")) if response and "result" in response else {}}}
    return {"protocol.json": codex_protocol(document), "launch-probe.json": probe}


def public_snapshot(provider: str, executable: Path) -> tuple[str, dict[str, object]]:
    executable = executable.resolve()
    if provider == "codex" and (len(executable.parents) < 3 or executable.parents[2].name != "vendor"):
        raise ValueError("codex snapshots must name the vendored native binary vendor/<triple>/bin/codex, not the npm shim")
    version_output = run_text([str(executable), "--version"]).strip()
    version = version_of(provider, version_output)
    with tempfile.TemporaryDirectory(prefix=f"gent-{provider}-contract-") as directory:
        scratch = Path(directory)
        files = {"cli.json": cli_surface(provider, executable)}
        files.update(claude_capture(executable, scratch) if provider == "claude" else codex_capture(executable, scratch))
    source = {"provider": provider, "version": version, "version_output": version_output, "executable_file": executable.name, "executable_sha256": sha256(executable), "evidence": "installedExecutable"}
    return version, {"source.json": source, **files}


def claurst_snapshot(checkout: Path, acp_schema: Path) -> tuple[str, dict[str, object]]:
    manifest = (checkout / "src-rust/Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r"^version\s*=\s*\"([^\"]+)\"", manifest, re.M)
    if match is None:
        raise ValueError("Claurst workspace version not found")
    commit = run_text(["git", "-C", str(checkout), "rev-parse", "HEAD"]).strip()
    source = {"provider": "claurst", "version": match.group(1), "evidence": "sourceCheckout", "commit": commit, "acp_schema": acp_schema.name, "pinned_release_binary_verified": False}
    protocol = claurst_protocol(checkout, acp_schema)
    return match.group(1), {"source.json": source, "protocol.json": protocol, "commands.json": {}}


def write_snapshot(directory: Path, files: dict[str, object]) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    stamp = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat()
    files["source.json"] = {**files["source.json"], "platform": f"{sys.platform}-{platform.machine()}", "captured_at": stamp, "tool": TOOL}
    for name, value in files.items():
        (directory / name).write_text(json.dumps(value, indent=1, sort_keys=True) + "\n", encoding="utf-8")


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Capture or diff a provider's machine-readable contract surface.")
    parser.add_argument("provider", nargs="?", choices=("claude", "codex", "claurst"))
    parser.add_argument("--executable", type=Path)
    parser.add_argument("--claurst-checkout", type=Path)
    parser.add_argument("--acp-schema", type=Path)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--diff", nargs=2, type=Path, metavar=("OLD", "NEW"))
    return parser.parse_args()


def main() -> int:
    args = arguments()
    if args.diff:
        findings = diff_snapshots(*(path if path.is_absolute() or path.exists() else CONTRACTS / path for path in args.diff))
        print("\n".join(findings) if findings else "no contract changes")
        return 1 if findings else 0
    if args.provider == "claurst":
        if args.claurst_checkout is None or args.acp_schema is None:
            raise ValueError("claurst snapshots need --claurst-checkout and --acp-schema")
        version, files = claurst_snapshot(args.claurst_checkout, args.acp_schema)
    elif args.provider and args.executable:
        version, files = public_snapshot(args.provider, args.executable)
    else:
        raise ValueError("pass a provider with --executable, claurst with its checkout, or --diff OLD NEW")
    target = args.out or CONTRACTS / args.provider / version
    write_snapshot(target, files)
    print(json.dumps({"provider": args.provider, "version": version, "directory": str(target), "files": sorted(files)}))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        raise SystemExit(f"provider contract snapshot failed: {error}") from error
