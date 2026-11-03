"""Portable producer metadata checks; no keys, signing, network or native calls."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
from runtime_release_policy import compatibility

spec = importlib.util.spec_from_file_location("release_signer_policy", Path(__file__).with_name("sign-runtime-release.py"))
signer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signer)

class ReleasePolicyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="gent-policy-fixture-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.policy = {"schemaVersion": 1, "minimumCompatibleHostVersion": "0.2.4",
                       "protocolMin": 1, "protocolMax": 1, "schemaMin": 1}
        (self.root / "runtime-release-policy.json").write_text(json.dumps(self.policy))
        source = self.root / "crates/gent-store/src/lib.rs"
        source.parent.mkdir(parents=True)
        source.write_text("pub const CURRENT_LEDGER_SCHEMA_VERSION: u32 = 26;\n")

    def test_current_schema_has_one_authoritative_source_and_missing_or_ambiguous_is_refused(self):
        self.assertEqual(compatibility(self.root)["schemaMax"], 26)
        source = self.root / "crates/gent-store/src/lib.rs"
        for content in ("", "pub const CURRENT_LEDGER_SCHEMA_VERSION: u32 = 26;\n" * 2):
            source.write_text(content)
            with self.assertRaisesRegex(ValueError, "authoritative current ledger"):
                compatibility(self.root)

    def test_new_archive_manifest_keeps_legacy_deny_unknown_fields_shape(self):
        path = Path(__file__).with_name("package-release.py")
        spec = importlib.util.spec_from_file_location("archive_packager", path)
        packager = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(packager)
        name = "gent-v0.2.5-fixture.tar.gz"
        (self.root / name).write_bytes(b"fake archive")
        packager.write_metadata(self.root, name, "v0.2.5", "fixture", [Path("gent"), Path("gentd")])
        manifest = json.loads((self.root / (name + ".manifest.json")).read_text())
        self.assertEqual(set(manifest), {"schemaVersion", "version", "target", "archive", "binaries", "capabilities", "runtimes"})
        self.assertEqual(manifest["schemaVersion"], 1)
        self.assertNotIn("cliCapabilities", manifest)

    def metadata(self, host_floor=None):
        policy = compatibility(self.root)
        if host_floor:
            policy["minimumCompatibleHostVersion"] = host_floor
        archive = self.root / "archive.json"
        archive.write_text(json.dumps({"schemaVersion": 1, "version": "v0.2.5", "target": "fixture",
            "archive": {"name": "fixture.tar.gz", "sha256": "a" * 64, "size": 1}}))
        arguments = ["signer", "--archive-manifest", str(archive), "--version", "v0.2.5",
            "--target", "fixture", "--key-id", "fixture", "--private-key", "never-read",
            "--expires-at", "4102444800", "--out", str(self.root / "never-written")]
        with patch.object(signer, "compatibility", return_value=policy), patch.object(sys, "argv", arguments):
            return signer.payload(signer.parse_args())

    def test_new_target_does_not_raise_compatible_host_floor_or_truncate_schema_range(self):
        payload = self.metadata()
        self.assertEqual(payload["minimumAppVersion"], {"major": 0, "minor": 2, "patch": 4})
        self.assertEqual(payload["releaseVersion"], {"major": 0, "minor": 2, "patch": 5})
        self.assertEqual((payload["schemaMin"], payload["schemaMax"]), (1, 26))
        for previous in (23, 26):
            self.assertLessEqual(payload["schemaMin"], previous)
            self.assertGreaterEqual(payload["schemaMax"], previous)

    def test_independent_unsupported_floor_stays_explicit_in_offer(self):
        payload = self.metadata("0.2.6")
        self.assertEqual(payload["minimumAppVersion"], {"major": 0, "minor": 2, "patch": 6})
        self.assertGreater(tuple(payload["minimumAppVersion"][part] for part in ("major", "minor", "patch")), (0, 2, 4))

    def test_invalid_compatibility_policy_cannot_silently_fall_back_to_target_version(self):
        for change in ({"minimumCompatibleHostVersion": "GUI-version"}, {"schemaMin": 27},
                       {"protocolMin": 2, "protocolMax": 1}, {"schemaMin": True}):
            (self.root / "runtime-release-policy.json").write_text(json.dumps(self.policy | change))
            with self.assertRaises(ValueError):
                compatibility(self.root)

if __name__ == "__main__":
    unittest.main()
