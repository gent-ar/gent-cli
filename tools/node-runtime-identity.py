#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from provider_pins import PINS, load_pins


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Check a staged Node runtime against its upstream pin, or record the digest of the Node binary a release ships")
    parser.add_argument("action", choices=("check-upstream", "record"))
    parser.add_argument("--target", required=True)
    parser.add_argument("--node-runtime-dir", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path)
    return parser.parse_args()


def node_sha256(runtime: Path, target: str) -> str:
    binary = runtime / "bin" / ("node.exe" if "-windows-" in target else "node")
    if not binary.is_file() or binary.is_symlink():
        raise ValueError(f"staged Node binary is missing: {binary}")
    return hashlib.sha256(binary.read_bytes()).hexdigest()


def check_upstream(runtime: Path, target: str) -> None:
    pinned = load_pins(PINS)["runtimes"]["node"]["artifacts"][target]["upstream_node_sha256"]
    observed = node_sha256(runtime, target)
    if observed != pinned:
        raise ValueError(f"staged Node for {target} is {observed}, not the pinned upstream {pinned}")


def record(runtime: Path, target: str, out_dir: Path) -> None:
    if target not in load_pins(PINS)["runtimes"]["node"]["artifacts"]:
        raise ValueError(f"{target} is not a pinned Node runtime target")
    out_dir.mkdir(parents=True, exist_ok=True)
    measurement = {"target": target, "node_sha256": node_sha256(runtime, target)}
    (out_dir / f"{target}.json").write_text(json.dumps(measurement) + "\n", encoding="utf-8")
    print(f"{target} ships Node sha256 {measurement['node_sha256']}")


def main() -> None:
    args = arguments()
    if args.action == "check-upstream":
        check_upstream(args.node_runtime_dir, args.target)
    elif args.out_dir is None:
        raise ValueError("record needs --out-dir")
    else:
        record(args.node_runtime_dir, args.target, args.out_dir)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        raise SystemExit(f"node runtime identity failed: {error}") from error
