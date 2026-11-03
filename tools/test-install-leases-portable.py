"""Tiny fake-file tests of installer transactions; no native binaries or OS leases."""
import argparse
from contextlib import contextmanager, ExitStack
import hashlib
import importlib.util
import os
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch
import activate_install_support as support

spec = importlib.util.spec_from_file_location("leased_installer", Path(__file__).with_name("activate-install.py"))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)

class InstallerLeaseTests(unittest.TestCase):
    def test_reader_accepts_operation_generation_without_broadening_other_lengths(self):
        for size in (8, 64):
            name = f"v1.2.3+{'a' * 16}.{'b' * size}-target"
            self.assertEqual(support.release_parts(name), ("v1.2.3", "target"))
        for size in (0, 7, 9, 63, 65):
            with self.assertRaises(SystemExit):
                support.release_parts(f"v1.2.3+{'a' * 16}.{'b' * size}-target")

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="gent-lease-fixture-")
        self.addCleanup(self.temp.cleanup)
        self.work = Path(self.temp.name)
        self.root, self.source = self.work / "store", self.work / "source"
        self.source.mkdir()
        for name in ("gent", "gentd"):
            (self.source / name).write_bytes(name.encode())
            (self.source / name).chmod(0o755)
        self.current = None
        self.events, self.busy, self.advance = [], False, None
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(patch.object(support, "selected_release", side_effect=lambda _: self.current))
        self.stack.enter_context(patch.object(installer, "selected_release", side_effect=lambda _: self.current))
        self.stack.enter_context(patch.object(installer, "require_executable", side_effect=support.require_regular_file))
        self.stack.enter_context(patch.object(installer, "mutation_lease", side_effect=self.lease))
        self.stack.enter_context(patch.object(installer, "select_release", side_effect=self.select))
        self.stack.enter_context(patch.object(installer, "fsync_directory"))

    @contextmanager
    def lease(self, root, data):
        self.events.append("data")
        if self.busy:
            raise SystemExit("fixture owner is busy")
        self.events.append("store")
        if self.advance:
            self.current = self.advance
        token = support.MutationLease()
        try:
            yield token
        finally:
            token.held = False
            self.events.append("release")

    def select(self, root, release, digest, lease):
        lease.require()
        self.assertEqual(support.tree_digest(release), digest)
        self.current = release.name
        self.events.append("select")

    def args(self, name="v1.2.3-target"):
        return argparse.Namespace(runtime_root=self.root, release_name=name,
            source_release=self.source, source_supervisor=None, source_auto_updater=None,
            source_update_material=None, bin_dir=self.work / "bin", idle_data_dir=self.work / "data",
            force=True, stage_only=False, rollback_from=None, expected_tree_sha256=None)

    def test_first_install_is_qualified_and_same_version_repair_never_mutates_old_tree(self):
        first = installer.install(self.args())
        before = support.tree_digest(first)
        self.assertRegex(first.name, r"^v1\.2\.3\+[0-9a-f]{16}\.[0-9a-f]{8}-target$")
        self.assertEqual(self.events[:2], ["data", "store"])
        (self.source / "gentd").write_bytes(b"repaired")
        second = installer.install(self.args())
        self.assertNotEqual(first, second)
        self.assertEqual(support.tree_digest(first), before)
        self.assertEqual((first / "gentd").read_bytes(), b"gentd")
        self.assertEqual((second / "gentd").read_bytes(), b"repaired")
        self.assertEqual(self.current, second.name)

    def test_equal_current_generation_is_reused_without_rewriting_it(self):
        first = installer.install(self.args())
        before = (first / "gentd").stat().st_mtime_ns
        second = installer.install(self.args())
        self.assertEqual(first, second)
        self.assertEqual((first / "gentd").stat().st_mtime_ns, before)

    def test_busy_owner_has_no_release_launcher_or_selected_mutation(self):
        self.root.mkdir()
        self.busy = True
        with self.assertRaisesRegex(SystemExit, "busy"):
            installer.install(self.args())
        self.assertFalse((self.root / "releases").exists())
        self.assertFalse((self.work / "bin").exists())
        self.assertIsNone(self.current)
        self.assertEqual(self.events, ["data"])
        self.assertFalse(list(self.root.glob(".gent-stage-*")))

    def test_newer_selection_is_refused_before_preparation_or_publication(self):
        self.current = "v9.0.0+0123456789abcdef.12345678-target"
        with patch.object(installer, "prepare_release") as prepare:
            with self.assertRaisesRegex(SystemExit, "newer runtime"):
                installer.install(self.args())
            prepare.assert_not_called()
        self.assertFalse(self.root.exists())

    def test_newer_selection_during_preparation_is_rechecked_under_both_leases(self):
        self.advance = "v9.0.0+0123456789abcdef.12345678-target"
        with self.assertRaisesRegex(SystemExit, "newer runtime"):
            installer.install(self.args())
        self.assertEqual(self.current, self.advance)
        self.assertFalse((self.root / "releases").exists())
        self.assertFalse((self.work / "bin").exists())
        self.assertFalse(list(self.root.glob(".gent-stage-*")))

    def test_tampered_private_copy_fails_before_any_mutation_lease(self):
        copy = installer.shutil.copytree
        def corrupt(source, destination, **options):
            result = copy(source, destination, **options)
            (destination / "gentd").write_bytes(b"corrupt private copy")
            return result
        with patch.object(installer.shutil, "copytree", side_effect=corrupt):
            with self.assertRaisesRegex(SystemExit, "source changed"):
                installer.install(self.args())
        self.assertEqual(self.events, [])
        self.assertFalse((self.root / "releases").exists())

    def test_rollback_is_bound_to_exact_current_generation_and_prior_hash(self):
        first = installer.install(self.args())
        digest = support.tree_digest(first)
        (self.source / "gentd").write_bytes(b"next")
        second = installer.install(self.args())
        args = self.args()
        args.source_release = None
        args.release_name = first.name
        args.rollback_from = "another-generation"
        args.expected_tree_sha256 = digest
        with self.assertRaisesRegex(SystemExit, "exact still-selected"):
            installer.activate_existing(args)
        self.assertEqual(self.current, second.name)
        args.rollback_from = second.name
        args.expected_tree_sha256 = "0" * 64
        with self.assertRaisesRegex(SystemExit, "verified identity"):
            installer.activate_existing(args)
        self.assertEqual(self.current, second.name)
        args.expected_tree_sha256 = digest
        restored = installer.activate_existing(args)
        self.assertEqual(self.current, restored.name)
        self.assertEqual(support.tree_digest(restored), digest)
        self.assertTrue(second.exists())

    def test_expired_mutation_token_cannot_publish(self):
        self.root.mkdir()
        stage, digest = installer.prepare_release(self.root, self.source, None, None, None)
        token = support.MutationLease()
        token.held = False
        with self.assertRaisesRegex(SystemExit, "not held"):
            installer.publish_release(self.root, "v1.2.3-target", stage, digest, token)
        self.assertFalse((self.root / "releases").exists())
        installer.remove_private_stage(self.root, stage)

    def test_tree_digest_matches_native_framing_including_utf8_sort_and_modes(self):
        root = self.work / "hash"
        root.mkdir()
        for name in ("\ue000", "\U0001f600", "a"):
            (root / name).write_bytes(name.encode())
        expected = hashlib.sha256(b"gent-runtime-tree-v1\0")
        for path in sorted(root.iterdir(), key=lambda value: value.name.encode("utf-8")):
            relative = path.name.encode("utf-8")
            mode = path.stat().st_mode & 0o777 if os.name != "nt" else 0
            expected.update(struct.pack(">Q", len(relative)) + relative
                            + hashlib.sha256(path.read_bytes()).digest() + struct.pack(">I", mode))
        self.assertEqual(support.tree_digest(root), expected.hexdigest())

    def test_lease_order_is_data_then_store_and_missing_explicit_data_fails_closed(self):
        order = []
        @contextmanager
        def lock(directory, name, busy):
            order.append(name)
            yield
        with patch.object(support, "file_lease", side_effect=lock):
            with support.mutation_lease(self.root, self.work / "data"):
                self.assertEqual(order, ["gentd.lock", ".install.lock"])
        self.current = "v1.2.3+0123456789abcdef.12345678-target"
        with self.assertRaisesRegex(SystemExit, "explicit --idle-data-dir"):
            with support.mutation_lease(self.root, None):
                self.fail("unscoped existing-runtime mutation was allowed")

if __name__ == "__main__":
    unittest.main()
