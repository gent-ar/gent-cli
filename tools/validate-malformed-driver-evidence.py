#!/usr/bin/env python3
"""Validate a redacted, provider-emitted malformed-frame evidence candidate.

This tool never starts a provider, proxies a stream, or writes a fixture.  It
keeps the malformed-tolerance matrix cells capture-required until a vendor
documents a bounded output-fault control.  Synthetic unit tests prove parser
behavior; this validator only checks the additional facts a future *live*
candidate must declare.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any


BOUNDARIES = frozenset((
    "structured_provider_frame",
    "unknown_provider_frame",
    "ndjson_transport_frame",
))
DIAGNOSTICS = {
    "claude": re.compile(r"^(?:malformedClaude|unsupportedClaude).+$"),
    "codex": re.compile(r"^(?:malformedCodex|unsupportedCodex).+$|^codexRpcError$"),
}
CONTRACT = {
    "capture": "vendor-documented bounded output-fault control",
    "session": "attended, read-only, tool-free, ephemeral",
    "source": "provider_emitted; never proxy, injection, replay, or shim",
    "continuation": "one ordinary provider frame after the diagnostic",
    "retention": "redacted structural shape and digest only; no raw output",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", type=Path, nargs="?")
    parser.add_argument("--describe", action="store_true")
    return parser.parse_args()


def load(path: Path) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    lines = [line for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    if len(lines) < 3:
        raise ValueError("fixture needs metadata, a fault frame, and a continuation frame")
    try:
        header = json.loads(lines[0])
        frames = [json.loads(line) for line in lines[1:]]
    except json.JSONDecodeError as error:
        raise ValueError(f"fixture must be valid redacted JSONL: {error.msg}") from error
    metadata = header.get("meta") if isinstance(header, dict) else None
    if not isinstance(metadata, dict) or not all(isinstance(frame, dict) for frame in frames):
        raise ValueError("fixture must have a meta object followed by object frames")
    return metadata, frames


def non_empty(value: object, field: str, errors: list[str]) -> None:
    if not isinstance(value, str) or not value.strip():
        errors.append(f"meta.{field} must be a non-empty string")


def metadata_errors(metadata: dict[str, Any]) -> tuple[str, list[str]]:
    errors: list[str] = []
    vendor = metadata.get("vendor")
    if vendor not in DIAGNOSTICS:
        errors.append("meta.vendor must be claude or codex")
        return "", errors
    if metadata.get("scenario") != "malformed_tolerance":
        errors.append("meta.scenario must be malformed_tolerance")
    if metadata.get("status") != "recorded" or metadata.get("captureOrigin") != "live_cli":
        errors.append("candidate must be a live recorded fixture")
    if metadata.get("faultSource") != "provider_emitted":
        errors.append("meta.faultSource must be provider_emitted")
    boundary = metadata.get("faultBoundary")
    if boundary not in BOUNDARIES:
        errors.append("meta.faultBoundary is not a supported parser boundary")
    if metadata.get("faultControlKind") != "vendor_documented":
        errors.append("meta.faultControlKind must be vendor_documented")
    for field in ("faultControl", "faultControlReference", "faultShapeDigest"):
        non_empty(metadata.get(field), field, errors)
    reference = metadata.get("faultControlReference")
    if isinstance(reference, str) and not reference.startswith("https://"):
        errors.append("meta.faultControlReference must be an https URL")
    digest = metadata.get("faultShapeDigest")
    if isinstance(digest, str) and not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
        errors.append("meta.faultShapeDigest must be a sha256 digest")
    return vendor, errors


def frame_errors(vendor: str, frames: list[dict[str, Any]]) -> list[str]:
    errors: list[str] = []
    fault_index = None
    for index, frame in enumerate(frames):
        fields = frame.get("expectFields")
        if frame.get("expect") != "transport_diagnostic" or not isinstance(fields, dict):
            continue
        classification = fields.get("classification")
        if isinstance(classification, str) and DIAGNOSTICS[vendor].fullmatch(classification):
            if frame.get("in") is None:
                errors.append("fault frame must retain a redacted structural in value")
            elif fields.get("providerEmitted") is not True:
                errors.append("fault frame must state providerEmitted=true")
            else:
                fault_index = index
                break
    if fault_index is None:
        errors.append("fixture needs one vendor-specific malformed/unknown diagnostic frame")
        return errors
    continuation = frames[fault_index + 1 :]
    if not any(
        frame.get("expect") != "transport_diagnostic"
        and isinstance(frame.get("expectFields"), dict)
        and frame["expectFields"].get("afterFault") is True
        for frame in continuation
    ):
        errors.append("fixture needs an ordinary provider frame marked afterFault=true")
    return errors


def validate(path: Path) -> None:
    metadata, frames = load(path)
    vendor, errors = metadata_errors(metadata)
    if vendor:
        errors.extend(frame_errors(vendor, frames))
    if errors:
        raise ValueError("\n".join(errors))


def main() -> int:
    args = parse_args()
    if args.describe:
        if args.fixture is not None:
            raise ValueError("--describe does not accept a fixture")
        print(json.dumps(CONTRACT, sort_keys=True, separators=(",", ":")))
        return 0
    if args.fixture is None:
        raise ValueError("fixture is required unless --describe is used")
    validate(args.fixture)
    print(f"valid malformed provider evidence candidate: {args.fixture}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:
        print(f"malformed provider evidence rejected: {error}", file=sys.stderr)
        sys.exit(1)
