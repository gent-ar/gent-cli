import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

from windows_sandbox_package import sha

SPEC = importlib.util.spec_from_file_location('prepare_model', Path(__file__).with_name('prepare-windows-model-manifest.py'))
PREPARE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPARE)


class ModelManifestPreparation(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.helper = self.root / 'helper.exe'
        self.worker = self.root / 'worker.exe'
        self.helper.write_bytes(b'new helper')
        self.worker.write_bytes(b'reviewed worker')
        self.template = self.root / 'template.json'
        self.output = self.root / 'result.json'
        self.model = {'schema': 1, 'transport': 'server-context-stdio-v1',
                      'helperSha256': 'a' * 64, 'workerSha256': sha(self.worker),
                      'osLibraries': [{'name': 'kernel32.dll', 'sha256': 'b' * 64}]}
        self.write_template()

    def write_template(self):
        self.template.write_text(json.dumps(self.model), encoding='utf-8')

    def run_prepare(self, helper_pin=None):
        return PREPARE.prepare(self.template, sha(self.template), self.helper,
                               helper_pin or sha(self.helper), self.worker, sha(self.worker),
                               self.output)

    def test_rebinds_only_helper_and_never_overwrites(self):
        self.assertEqual(self.run_prepare(), sha(self.output))
        result = json.loads(self.output.read_text())
        self.assertEqual(result['helperSha256'], sha(self.helper))
        self.assertEqual(result['workerSha256'], self.model['workerSha256'])
        self.assertEqual(result['osLibraries'], self.model['osLibraries'])
        with self.assertRaises(FileExistsError):
            self.run_prepare()

    def test_rejects_unreviewed_helper_and_worker_mismatch(self):
        with self.assertRaises(ValueError):
            self.run_prepare('0' * 64)
        self.model['workerSha256'] = '0' * 64
        self.write_template()
        with self.assertRaises(ValueError):
            self.run_prepare()
        self.assertFalse(self.output.exists())

    def test_rejects_duplicate_or_invalid_os_dependencies(self):
        self.model['osLibraries'].append({'name': 'KERNEL32.DLL', 'sha256': 'c' * 64})
        self.write_template()
        with self.assertRaises(ValueError):
            self.run_prepare()
        self.assertFalse(self.output.exists())


if __name__ == '__main__':
    unittest.main()
