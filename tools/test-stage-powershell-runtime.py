import importlib.util
from pathlib import Path
import struct
import tempfile
import unittest
import zipfile


SPEC = importlib.util.spec_from_file_location('stager', Path(__file__).with_name('stage-powershell-runtime.py'))
STAGER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STAGER)


class RuntimeStaging(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.archive = self.root / 'fixture.zip'
        self.output = self.root / 'runtime'

    def package(self, extra=(), omit=()):
        image = bytearray(256)
        image[:2] = b'MZ'
        struct.pack_into('<I', image, 60, 128)
        image[128:134] = b'PE\0\0d\x86'
        with zipfile.ZipFile(self.archive, 'w') as package:
            for name in sorted(STAGER.REQUIRED - set(omit)):
                package.writestr(name, image if name == 'pwsh.exe' else b'fixture')
            for name in extra:
                entry = zipfile.ZipInfo(name)
                entry.filename = name
                package.writestr(entry, b'fixture-extra')
        return STAGER.sha(self.archive)

    def test_exact_archive_yields_finite_files_and_no_acceptance_claim(self):
        digest = self.package(extra=['Modules/Fixture/Fixture.psd1'])
        proof = STAGER.stage(self.archive, digest, '7.6.9', self.output)
        self.assertEqual(len(STAGER.REQUIRED) + 1, proof['fileCount'])
        self.assertFalse(proof['installed'])
        self.assertFalse(proof['psecAccepted'])
        for record in proof['files']:
            self.assertEqual('powerShellRuntime', record['root'])
            self.assertEqual(record['sha256'], STAGER.sha(self.output / record['path']))

    def test_changed_archive_is_rejected_before_output(self):
        self.package()
        with self.assertRaisesRegex(ValueError, 'reviewed identity'):
            STAGER.stage(self.archive, '0' * 64, '7.6.9', self.output)
        self.assertFalse(self.output.exists())

    def test_existing_output_is_preserved(self):
        digest = self.package()
        self.output.mkdir()
        (self.output / 'keep').write_bytes(b'original')
        with self.assertRaisesRegex(ValueError, 'must be new'):
            STAGER.stage(self.archive, digest, '7.6.9', self.output)
        self.assertEqual(b'original', (self.output / 'keep').read_bytes())

    def test_paths_and_case_collisions_are_rejected_before_writes(self):
        for name in ['../escape', 'C:/escape', 'pwsh.EXE', 'Modules/NUL.txt', 'Modules/trailing.', 'Modules\\escape']:
            digest = self.package(extra=[name])
            with self.assertRaises(ValueError):
                STAGER.stage(self.archive, digest, '7.6.9', self.output)
            self.assertFalse(self.output.exists(), name)

    def test_missing_core_runtime_is_not_packaged(self):
        digest = self.package(omit=['coreclr.dll'])
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            STAGER.stage(self.archive, digest, '7.6.9', self.output)
        self.assertFalse(self.output.exists())


if __name__ == '__main__':
    unittest.main()
