#!/usr/bin/env python3
from __future__ import annotations

import base64
import hashlib
import io
import json
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

from provider_pins import PINS, authority_mismatches, load_pins, platform_tarball_digests, require_pinned_authority, tarball_mismatches
from provider_pins_testing import pinned_payload, write_pins


def mismatches(payload: dict[str, object], snapshots: bool = True) -> list[str]:
    with tempfile.TemporaryDirectory() as directory:
        return authority_mismatches(payload, write_pins(Path(directory), snapshots))


def entry(payload: dict[str, object], document: str, predicate) -> dict[str, object]:
    return next(item for item in payload[document]["payload"]["entries"] if predicate(item))


def test_pinned_payload_is_accepted() -> None:
    assert mismatches(pinned_payload()) == []


def test_compatibility_binds_each_target_to_its_native_binary_digest_and_version() -> None:
    payload = pinned_payload()
    entry(payload, "compatibility", lambda item: item["id"] == "codex-9.2.0-linux-x64")["digest_sha256"] = "e" * 64
    entry(payload, "compatibility", lambda item: item["id"] == "claude-9.1.0-darwin-arm64")["version"] = "9.1.1 (Claude Code)"
    errors = mismatches(payload)
    assert any(error.startswith("compatibility codex-9.2.0-linux-x64 'codex-cli 9.2.0' 'eeee") for error in errors), errors
    assert any(error.startswith("compatibility claude-9.1.0-darwin-arm64 '9.1.1 (Claude Code)'") for error in errors), errors


def test_a_shim_or_untargeted_compatibility_entry_is_refused() -> None:
    payload = pinned_payload()
    payload["compatibility"]["payload"]["entries"].append(
        {"id": "codex-9.2.0", "provider": "codex", "version": "codex-cli 9.2.0", "digest_sha256": "f" * 64, "revoked": False}
    )
    assert "compatibility entry 'codex-9.2.0' is not a pinned provider target" in mismatches(payload)


def test_package_policy_must_be_the_pinned_platform_tarball() -> None:
    payload = pinned_payload()
    shim = entry(payload, "package_policy", lambda item: item["version"] == "9.2.0-darwin-arm64")
    shim["version"] = "9.2.0"
    errors = mismatches(payload)
    assert "package policy codex @openai/codex@9.2.0 does not match a pinned package" in errors, errors
    assert "package policy has no active entry for pinned codex @openai/codex@9.2.0-darwin-arm64" in errors, errors


def test_every_pinned_target_needs_active_entries() -> None:
    payload = pinned_payload()
    entry(payload, "compatibility", lambda item: item["id"] == "codex-9.2.0-darwin-arm64")["revoked"] = True
    errors = mismatches(payload)
    assert "compatibility has no active entry codex-9.2.0-darwin-arm64" in errors, errors


def test_pinned_versions_require_a_snapshot_of_a_pinned_target_binary() -> None:
    errors = mismatches(pinned_payload(), snapshots=False)
    assert any("pinned claude 9.1.0 has no fixtures/provider-contracts/claude/9.1.0/source.json snapshot" in error for error in errors), errors
    with tempfile.TemporaryDirectory() as directory:
        path = write_pins(Path(directory))
        (path.parent / "codex" / "9.2.0" / "source.json").write_text(
            json.dumps({"executable_sha256": "0" * 64, "version_output": "codex-cli 9.2.0"}), encoding="utf-8"
        )
        errors = authority_mismatches(pinned_payload(), path)
        assert errors == ["pinned codex 9.2.0 snapshot executable is not a pinned target binary"], errors


def test_require_raises_with_every_mismatch() -> None:
    payload = pinned_payload()
    payload["package_policy"]["payload"]["entries"] = []
    with tempfile.TemporaryDirectory() as directory:
        try:
            require_pinned_authority(payload, write_pins(Path(directory)))
        except ValueError as error:
            assert "does not match provider pins" in str(error)
        else:
            raise AssertionError("mismatched authority payload was accepted")


def test_generated_entries_match_the_pins_the_signer_enforces() -> None:
    with tempfile.TemporaryDirectory() as directory:
        pins = write_pins(Path(directory))
        tool = Path(__file__).with_name("provider-authority-entries.py")
        result = subprocess.run([sys.executable, tool, "--pins", pins, "--node-runtime-digest", "e" * 64, "--terms-version", "terms-2"], capture_output=True, text=True, check=True)
        entries = json.loads(result.stdout)
        payload = pinned_payload()
        payload["compatibility"]["payload"]["entries"] = entries["compatibility"]
        payload["package_policy"]["payload"]["entries"] = entries["package_policy"]
        assert authority_mismatches(payload, pins) == []
        assert {entry["node_runtime_digest_sha256"] for entry in entries["package_policy"]} == {"e" * 64}
        assert [entry["id"] for entry in entries["compatibility"]] == ["claude-9.1.0-darwin-arm64", "codex-9.2.0-darwin-arm64", "codex-9.2.0-linux-x64"]


def test_repository_pins_have_contract_snapshots_of_the_executed_native_binaries() -> None:
    pins = load_pins(PINS)
    assert pins["reconciliation"]["state"] == "ownerReconciliationRequired"
    for provider in ("claude", "codex"):
        pin = pins["providers"][provider]
        source = json.loads((PINS.parent / provider / pin["version"] / "source.json").read_text(encoding="utf-8"))
        assert source["executable_sha256"] == pin["targets"]["darwin-arm64"]["executable_sha256"], provider
        assert source["version_output"] == pin["version_output"], provider
    codex = pins["providers"]["codex"]["targets"]["darwin-arm64"]
    assert codex["package"]["version"] == pins["providers"]["codex"]["version"] + "-darwin-arm64"
    assert codex["executable"] == "@openai/codex/vendor/aarch64-apple-darwin/bin/codex"
    assert pins["providers"]["claurst"]["release_tag"] == "v" + pins["providers"]["claurst"]["version"]


def platform_tarball(root: Path, members: dict[str, bytes]) -> Path:
    path = root / "codex-9.2.0-darwin-arm64.tgz"
    with tarfile.open(path, "w:gz") as archive:
        for name, data in members.items():
            info = tarfile.TarInfo(name)
            info.size = len(data)
            archive.addfile(info, io.BytesIO(data))
    return path


def pinned_tarball_pins(root: Path, tarball: Path, native: bytes) -> dict[str, object]:
    pins = load_pins(write_pins(root))
    target = pins["providers"]["codex"]["targets"]["darwin-arm64"]
    target["package"]["integrity"] = "sha512-" + base64.b64encode(hashlib.sha512(tarball.read_bytes()).digest()).decode()
    target["executable_sha256"] = hashlib.sha256(native).hexdigest()
    return pins


def test_native_digest_is_the_vendored_binary_not_the_shim() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        native = b"native codex binary"
        tarball = platform_tarball(root, {"package/bin/codex.js": b"shim", "package/vendor/aarch64-apple-darwin/bin/codex": native})
        digests = platform_tarball_digests(tarball, "@openai/codex", "@openai/codex/vendor/aarch64-apple-darwin/bin/codex")
        assert digests["executable_sha256"] == hashlib.sha256(native).hexdigest()
        pins = pinned_tarball_pins(root, tarball, native)
        assert tarball_mismatches(tarball, "codex", "darwin-arm64", pins) == []
        pins["providers"]["codex"]["targets"]["darwin-arm64"]["executable_sha256"] = hashlib.sha256(b"shim").hexdigest()
        errors = tarball_mismatches(tarball, "codex", "darwin-arm64", pins)
        assert len(errors) == 1 and errors[0].startswith("codex darwin-arm64 executable_sha256"), errors


def test_native_digest_refuses_paths_outside_the_package_and_missing_binaries() -> None:
    with tempfile.TemporaryDirectory() as directory:
        tarball = platform_tarball(Path(directory), {"package/bin/codex.js": b"shim"})
        for executable in ("@openai/other/vendor/codex", "@openai/codex/../codex"):
            try:
                platform_tarball_digests(tarball, "@openai/codex", executable)
            except ValueError:
                pass
            else:
                raise AssertionError(f"{executable} was accepted")
        try:
            platform_tarball_digests(tarball, "@openai/codex", "@openai/codex/vendor/aarch64-apple-darwin/bin/codex")
        except KeyError:
            pass
        else:
            raise AssertionError("a tarball without the native binary was accepted")


def test_native_digest_check_exits_nonzero_on_a_mismatched_tarball() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        tarball = platform_tarball(root, {"package/vendor/aarch64-apple-darwin/bin/codex": b"native"})
        pins = root / "provider-contracts" / "pins.json"
        write_pins(root)
        tool = Path(__file__).with_name("provider-native-digest.py")
        result = subprocess.run([sys.executable, tool, "codex", "darwin-arm64", tarball, "--pins", pins, "--check"], capture_output=True, text=True)
        assert result.returncode == 1, result
        assert "codex darwin-arm64 integrity" in result.stderr and "executable_sha256" in result.stderr, result.stderr


if __name__ == "__main__":
    for name, test in sorted(globals().items()):
        if name.startswith("test_"):
            test()
    print("provider pin checks passed")
