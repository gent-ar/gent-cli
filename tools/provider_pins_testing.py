from __future__ import annotations

import copy
import json
from pathlib import Path

from provider_pins import pinned_entries


CLAUDE_DIGEST = "a" * 64
CODEX_DIGEST = "b" * 64
NODE_DIGESTS = {"darwin-arm64": "d" * 64, "linux-x64": "e" * 64}
PINS = {
    "schema_version": 2,
    "runtimes": {
        "node": {
            "version": "22.17.0",
            "artifacts": {
                "aarch64-apple-darwin": {"provider_target": "darwin-arm64", "upstream_node_sha256": "1" * 64},
                "x86_64-unknown-linux-gnu": {"provider_target": "linux-x64", "upstream_node_sha256": "2" * 64},
            },
        },
    },
    "providers": {
        "claude": {
            "version": "9.1.0",
            "version_output": "9.1.0 (Claude Code)",
            "targets": {
                "darwin-arm64": {
                    "package": {"name": "@anthropic-ai/claude-code-darwin-arm64", "version": "9.1.0", "integrity": "sha512-claude"},
                    "executable": "@anthropic-ai/claude-code-darwin-arm64/claude",
                    "executable_sha256": CLAUDE_DIGEST,
                },
            },
        },
        "codex": {
            "version": "9.2.0",
            "version_output": "codex-cli 9.2.0",
            "targets": {
                "darwin-arm64": {
                    "package": {"name": "@openai/codex", "version": "9.2.0-darwin-arm64", "integrity": "sha512-codex-arm"},
                    "executable": "@openai/codex/vendor/aarch64-apple-darwin/bin/codex",
                    "executable_sha256": CODEX_DIGEST,
                },
                "linux-x64": {
                    "package": {"name": "@openai/codex", "version": "9.2.0-linux-x64", "integrity": "sha512-codex-linux"},
                    "executable": "@openai/codex/vendor/x86_64-unknown-linux-musl/bin/codex",
                    "executable_sha256": "c" * 64,
                },
            },
        },
    },
}


def write_pins(root: Path, snapshots: bool = True) -> Path:
    contracts = root / "provider-contracts"
    contracts.mkdir(parents=True)
    path = contracts / "pins.json"
    path.write_text(json.dumps(PINS), encoding="utf-8")
    if snapshots:
        for provider, pin in PINS["providers"].items():
            snapshot = contracts / provider / pin["version"]
            snapshot.mkdir(parents=True)
            source = {"executable_sha256": pin["targets"]["darwin-arm64"]["executable_sha256"], "version_output": pin["version_output"]}
            (snapshot / "source.json").write_text(json.dumps(source), encoding="utf-8")
    return path


def envelope(entries: list[dict[str, object]]) -> dict[str, object]:
    return {"key_id": "k", "payload": {"entries": entries}, "signature_hex": "00"}


def pinned_payload() -> dict[str, object]:
    entries = pinned_entries(PINS, NODE_DIGESTS, "terms-1")
    compatibility, packages = entries["compatibility"], entries["package_policy"]
    return copy.deepcopy({
        "version": 1,
        "expires_at_unix_seconds": 100,
        "revoked": False,
        "compatibility": envelope(compatibility),
        "compatibility_keys": [{"key_id": "k"}],
        "package_policy": envelope(packages),
        "package_policy_keys": [{"key_id": "k"}],
        "providers": [{"provider": "claude"}, {"provider": "codex"}],
    })
