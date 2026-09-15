#!/usr/bin/env python3
from __future__ import annotations

import base64
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from provider_pins_testing import pinned_payload, write_pins


ROOT = Path(__file__).resolve().parent.parent
METADATA_SIGNER = ROOT / "tools/sign-runtime-release-metadata.sh"
AUTHORITY_VERIFIER = ROOT / "tools/verify-staged-authority.py"
SEED = bytes(range(32))


def release_signer():
    spec = importlib.util.spec_from_file_location("runtime_release_signer", ROOT / "tools/sign-runtime-release.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def private_key(seed: bytes) -> str:
    der = bytes.fromhex("302e020100300506032b657004220420") + seed
    return "-----BEGIN PRIVATE KEY-----\n" + base64.b64encode(der).decode() + "\n-----END PRIVATE KEY-----\n"


def shim(directory: Path, name: str, body: str) -> None:
    path = directory / name
    path.write_text(f"#!/bin/sh\n{body}\n", encoding="utf-8")
    path.chmod(0o755)


def shims(directory: Path) -> Path:
    directory.mkdir()
    shim(directory, "python", f'exec "{sys.executable}" "$@"')
    shim(directory, "cosign", 'test "$1 $2 $3" = "sign-blob --yes --bundle" && printf signed > "$4"')
    gnu_date = subprocess.run(["date", "-u", "--date", "+90 days", "+%s"], capture_output=True)
    if gnu_date.returncode != 0:
        shim(directory, "date", f'exec "{sys.executable}" -c "import time; print(int(time.time()) + 90 * 86400)"')
    return directory


def run_metadata_signer(root: Path, public_key: str) -> subprocess.CompletedProcess[str]:
    environment = {
        **os.environ,
        "PATH": f"{root / 'bin'}{os.pathsep}{os.environ['PATH']}",
        "RUNNER_TEMP": str(root / "runner-temp"),
        "RELEASE_TAG": "v1.2.3",
        "RUNTIME_RELEASE_KEY_ID": "release-1",
        "RUNTIME_RELEASE_PRIVATE_KEY": private_key(SEED).rstrip("\n"),
        "RUNTIME_RELEASE_PUBLIC_KEY": public_key,
    }
    return subprocess.run(["bash", str(METADATA_SIGNER)], cwd=root, env=environment, capture_output=True, text=True)


def write_archive_manifests(root: Path) -> None:
    for target in ("fixture-b", "fixture-a"):
        artifact = root / "release-artifacts" / f"release-{target}"
        artifact.mkdir(parents=True)
        (artifact / f"gent-v1.2.3-{target}.tar.gz.manifest.json").write_text(json.dumps({
            "schemaVersion": 1, "version": "v1.2.3", "target": target,
            "archive": {"name": f"gent-v1.2.3-{target}.tar.gz", "sha256": "a" * 64, "size": 1},
        }), encoding="utf-8")


def check_metadata_signer() -> None:
    public_key = release_signer().public_key(SEED).hex()
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        shims(root / "bin")
        (root / "runner-temp").mkdir()
        write_archive_manifests(root)
        result = run_metadata_signer(root, public_key)
        assert result.returncode == 0, result.stderr
        artifacts = root / "release-artifacts"
        releases = sorted(artifacts.rglob("*.runtime-release.json"))
        assert [path.name for path in releases] == [
            "gent-v1.2.3-fixture-a.runtime-release.json",
            "gent-v1.2.3-fixture-b.runtime-release.json",
        ]
        release = json.loads(releases[0].read_text())["payload"]
        assert release["schemaMax"] == 23 and release["minimumAppVersion"] == {"major": 1, "minor": 2, "patch": 3}
        index = artifacts / "gent-v1.2.3.runtime-release-index.json"
        offers = json.loads(index.read_text())["payload"]["offers"]
        assert [offer["target"] for offer in offers] == ["fixture-a", "fixture-b"]
        trust = artifacts / "gent-runtime-release-trust.json"
        assert trust.read_text() == json.dumps(
            {"schemaVersion": 1, "keys": [{"keyId": "release-1", "publicKeyHex": public_key}]},
            separators=(",", ":"),
        ) + "\n"
        for signed in [*releases, index, trust]:
            assert Path(f"{signed}.sigstore.json").read_text() == "signed"
        assert not list(artifacts.rglob("*.manifest.json.sigstore.json"))
        assert not (root / "runner-temp/gent-runtime-release.pem").exists()
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        shims(root / "bin")
        (root / "runner-temp").mkdir()
        write_archive_manifests(root)
        for mismatched in ("0" * 64, "A" * 64, public_key[:-1]):
            assert run_metadata_signer(root, mismatched).returncode != 0
        assert not list((root / "release-artifacts").rglob("*.runtime-release*.json"))


def check_authority_verifier() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        authority = root / "dist-runtime/authority"
        authority.mkdir(parents=True)
        (authority / "ordinary-authority.json").write_text(json.dumps({"key_id": "root", "payload": pinned_payload(), "signature_hex": "00"}), encoding="utf-8")
        pins = write_pins(root / "fixtures")
        (authority / "root-keys.json").write_text(json.dumps({"keys": ["one:01", "two:02"]}), encoding="utf-8")
        record = root / "record.json"
        recorder = root / "recorder.py"
        recorder.write_text(
            "import json, os, pathlib, sys\n"
            f"pathlib.Path({str(record)!r}).write_text(json.dumps({{'argv': sys.argv[1:], "
            "'node': os.environ['GENT_NODE_BINARY'], 'data': os.path.isdir(sys.argv[-1])}))\n",
            encoding="utf-8",
        )
        if os.name == "nt":
            verifier = root / "gentd.cmd"
            verifier.write_text(f'@"{sys.executable}" "{recorder}" %*\n', encoding="utf-8")
        else:
            verifier = root / "gentd"
            shim(root, "gentd", f'exec "{sys.executable}" "{recorder}" "$@"')
        environment = {**os.environ, "GENTD_VERIFIER": verifier.name, "NODE_BINARY": "node-runtime/node"}
        subprocess.run([sys.executable, str(AUTHORITY_VERIFIER), "--pins", str(pins)], cwd=root, env=environment, check=True)
        value = json.loads(record.read_text())
        release = str((authority / "ordinary-authority.json").resolve())
        assert value["argv"][:-2] == [
            "--standalone-authority", "--verify-standalone-authority-release",
            "--standalone-authority-release", release,
            "--standalone-authority-key", "one:01", "--standalone-authority-key", "two:02",
        ]
        assert value["argv"][-2] == "--data-dir" and value["data"]
        assert value["node"] == str((root / "node-runtime/node").resolve())
        failing = {**environment, "GENTD_VERIFIER": "missing-gentd"}
        missing = subprocess.run([sys.executable, str(AUTHORITY_VERIFIER), "--pins", str(pins)], cwd=root, env=failing, capture_output=True)
        assert missing.returncode != 0
        record.unlink()
        drifted = pinned_payload()
        drifted["package_policy"]["payload"]["entries"][0]["version"] = "0.0.1"
        (authority / "ordinary-authority.json").write_text(json.dumps({"key_id": "root", "payload": drifted, "signature_hex": "00"}), encoding="utf-8")
        refused = subprocess.run([sys.executable, str(AUTHORITY_VERIFIER), "--pins", str(pins)], cwd=root, env=environment, capture_output=True, text=True)
        assert refused.returncode != 0
        assert "does not match a pinned package" in refused.stderr, refused.stderr
        assert not record.exists()
        shim_digest = pinned_payload()
        shim_digest["compatibility"]["payload"]["entries"][1]["digest_sha256"] = "f" * 64
        (authority / "ordinary-authority.json").write_text(json.dumps({"key_id": "root", "payload": shim_digest, "signature_hex": "00"}), encoding="utf-8")
        refused = subprocess.run([sys.executable, str(AUTHORITY_VERIFIER), "--pins", str(pins)], cwd=root, env=environment, capture_output=True, text=True)
        assert refused.returncode != 0
        assert "compatibility codex-9.2.0-darwin-arm64 'codex-cli 9.2.0' 'ffff" in refused.stderr, refused.stderr
        assert not record.exists()


def main() -> None:
    check_authority_verifier()
    if os.name != "nt" and shutil.which("bash"):
        check_metadata_signer()
    print("release workflow script checks passed")


if __name__ == "__main__":
    main()
