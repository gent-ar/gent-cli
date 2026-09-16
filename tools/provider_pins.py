from __future__ import annotations

import base64
import hashlib
import json
import tarfile
from pathlib import Path


PINS = Path(__file__).resolve().parents[1] / "fixtures" / "provider-contracts" / "pins.json"
SIGNED_PROVIDERS = ("claude", "codex")


def load_pins(path: Path = PINS) -> dict[str, object]:
    pins = json.loads(path.read_text(encoding="utf-8"))
    if pins.get("schema_version") != 2 or not isinstance(pins.get("providers"), dict):
        raise ValueError(f"{path} is not a provider pin file v2")
    return pins


def entry_id(provider: str, pin: dict[str, object], target: str) -> str:
    return f"{provider}-{pin['version']}-{target}"


def pinned_targets(pins: dict[str, object]):
    for provider in SIGNED_PROVIDERS:
        pin = pins["providers"].get(provider)
        if pin is None:
            continue
        for target, detail in pin["targets"].items():
            yield provider, pin, target, detail


def pinned_entries(pins: dict[str, object], node_runtime_digests: dict[str, str], terms_version: str) -> dict[str, list[dict[str, object]]]:
    targets = list(pinned_targets(pins))
    unbound = sorted({target for _, _, target, _ in targets} - set(node_runtime_digests))
    if unbound:
        raise ValueError("pinned provider targets have no measured Node runtime digest: " + ", ".join(unbound))
    return {
        "compatibility": [
            {"id": entry_id(provider, pin, target), "provider": provider, "version": pin["version_output"], "digest_sha256": detail["executable_sha256"], "revoked": False}
            for provider, pin, target, detail in targets
        ],
        "package_policy": [
            {"provider": provider, "package_name": detail["package"]["name"], "version": detail["package"]["version"], "integrity": detail["package"]["integrity"], "node_runtime_digest_sha256": node_runtime_digests[target], "terms_version": terms_version, "revoked": False}
            for provider, _, target, detail in targets
        ],
    }


def active_entries(document: object, label: str) -> list[dict[str, object]]:
    if not isinstance(document, dict) or not isinstance(document.get("payload"), dict):
        raise ValueError(f"{label} is not a signed envelope")
    entries = document["payload"].get("entries")
    if not isinstance(entries, list) or not all(isinstance(entry, dict) for entry in entries):
        raise ValueError(f"{label} entries are malformed")
    return [entry for entry in entries if entry.get("revoked") is not True]


def compatibility_mismatches(payload: dict[str, object], pins: dict[str, object]) -> list[str]:
    entries = active_entries(payload.get("compatibility"), "compatibility")
    expected = {
        entry_id(provider, pin, target): (provider, pin["version_output"], detail["executable_sha256"])
        for provider, pin, target, detail in pinned_targets(pins)
    }
    errors = []
    for entry in entries:
        observed = (entry.get("provider"), entry.get("version"), entry.get("digest_sha256"))
        pinned = expected.get(entry.get("id"))
        if pinned is None:
            errors.append(f"compatibility entry {entry.get('id')!r} is not a pinned provider target")
        elif observed != pinned:
            errors.append(f"compatibility {entry.get('id')} {observed[1]!r} {observed[2]!r} is not pinned {pinned[1]!r} {pinned[2]!r}")
    present = {entry.get("id") for entry in entries}
    errors.extend(f"compatibility has no active entry {identifier}" for identifier in sorted(set(expected) - present))
    return errors


def package_mismatches(payload: dict[str, object], pins: dict[str, object]) -> list[str]:
    entries = active_entries(payload.get("package_policy"), "package_policy")
    expected = {
        (provider, detail["package"]["name"], detail["package"]["version"], detail["package"]["integrity"])
        for provider, _, _, detail in pinned_targets(pins)
    }
    observed = {(entry.get("provider"), entry.get("package_name"), entry.get("version"), entry.get("integrity")) for entry in entries}
    errors = [f"package policy {item[0]} {item[1]}@{item[2]} does not match a pinned package" for item in sorted(observed - expected, key=str)]
    errors.extend(f"package policy has no active entry for pinned {item[0]} {item[1]}@{item[2]}" for item in sorted(expected - observed, key=str))
    return errors


def snapshot_mismatches(pins_path: Path, pins: dict[str, object]) -> list[str]:
    errors = []
    for provider in SIGNED_PROVIDERS:
        pin = pins["providers"].get(provider)
        if pin is None:
            continue
        source = pins_path.parent / provider / pin["version"] / "source.json"
        if not source.is_file():
            errors.append(f"pinned {provider} {pin['version']} has no fixtures/provider-contracts/{provider}/{pin['version']}/source.json snapshot")
            continue
        snapshot = json.loads(source.read_text(encoding="utf-8"))
        digests = {detail["executable_sha256"] for detail in pin["targets"].values()}
        if snapshot.get("executable_sha256") not in digests or snapshot.get("version_output") != pin["version_output"]:
            errors.append(f"pinned {provider} {pin['version']} snapshot executable is not a pinned target binary")
    return errors


def authority_mismatches(payload: dict[str, object], pins_path: Path = PINS) -> list[str]:
    pins = load_pins(pins_path)
    return (
        compatibility_mismatches(payload, pins)
        + package_mismatches(payload, pins)
        + snapshot_mismatches(pins_path, pins)
    )


def require_pinned_authority(payload: dict[str, object], pins_path: Path = PINS) -> None:
    errors = authority_mismatches(payload, pins_path)
    if errors:
        raise ValueError("authority payload does not match provider pins: " + "; ".join(errors))


def tarball_executable_member(package_name: str, executable: str) -> str:
    prefix = package_name + "/"
    if not executable.startswith(prefix) or ".." in executable.split("/"):
        raise ValueError(f"pinned executable {executable!r} is not inside package {package_name!r}")
    return "package/" + executable[len(prefix):]


def platform_tarball_digests(tarball: Path, package_name: str, executable: str) -> dict[str, str]:
    data = tarball.read_bytes()
    member = tarball_executable_member(package_name, executable)
    with tarfile.open(tarball, "r:gz") as archive:
        entry = archive.getmember(member)
        binary = archive.extractfile(entry) if entry.isfile() else None
        if binary is None:
            raise ValueError(f"{tarball} member {member} is not a regular file")
        executable_sha256 = hashlib.sha256(binary.read()).hexdigest()
    return {
        "integrity": "sha512-" + base64.b64encode(hashlib.sha512(data).digest()).decode("ascii"),
        "executable_sha256": executable_sha256,
    }


def tarball_mismatches(tarball: Path, provider: str, target: str, pins: dict[str, object]) -> list[str]:
    detail = pins["providers"][provider]["targets"][target]
    observed = platform_tarball_digests(tarball, detail["package"]["name"], detail["executable"])
    pinned = {"integrity": detail["package"]["integrity"], "executable_sha256": detail["executable_sha256"]}
    return [
        f"{provider} {target} {field} {observed[field]} is not pinned {pinned[field]}"
        for field in ("integrity", "executable_sha256")
        if observed[field] != pinned[field]
    ]
