import importlib.util
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

SPEC = importlib.util.spec_from_file_location('check_release', Path(__file__).with_name('check-windows-sandbox-release.py'))
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


class WindowsReleaseGate(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.manifest = {'version': 'v1.2.3', 'target': 'x86_64-pc-windows-msvc',
                         'archive': {'name': 'release.zip'}, 'runtimes': ['runtime/windows']}
        self.required = ['provider-helper.exe', 'provider-resources.json',
                         'reviewed-external-proxy-policy.json', 'proxy-package.msix',
                         'git/etc/fstab', 'git/usr/bin/bash.exe', 'git/usr/bin/msys-2.0.dll']

    def write(self, missing=None):
        (self.root / 'release.manifest.json').write_text(json.dumps(self.manifest))
        sha = lambda value: hashlib.sha256(value).hexdigest()
        helper, resources, worker = b'fixture-helper', b'fixture-resources', b'fixture-worker'
        policy = {'schema': 2, 'providers': ['codex', 'claude', 'claurst'], 'policy': 'fixture',
                  'ingress': 'fixture', 'physicalLanVerified': False, 'helperSha256': sha(helper),
                  'resourcesSha256': sha(resources), 'package': {}}
        model = {'helperSha256': sha(helper), 'workerSha256': sha(worker)}
        payloads = {'provider-helper.exe': helper, 'provider-resources.json': resources,
                    'reviewed-external-proxy-policy.json': json.dumps(policy).encode(),
                    'git/etc/fstab': b'none /tmp usertemp binary,posix=0 0 0\nnone / cygdrive binary,posix=0,user 0 0\n'}
        with zipfile.ZipFile(self.root / 'release.zip', 'w') as package:
            for name in self.required:
                if name != missing:
                    package.writestr('gent/runtime/windows/' + name, payloads.get(name, b'fixture'))
            package.writestr('gent/runtime/claurst/model-server/model-runtime.json', json.dumps(model))
            package.writestr('gent/runtime/claurst/model-server/gent-contained-model-factory.exe', worker)

    def test_accepts_complete_layout(self):
        self.write()
        CHECK.check(self.root, 'v1.2.3', 'x86_64-pc-windows-msvc')

    def test_rejects_missing_mount_or_runtime_declaration(self):
        self.write(missing='git/etc/fstab')
        with self.assertRaisesRegex(ValueError, 'fstab'):
            CHECK.check(self.root, 'v1.2.3', 'x86_64-pc-windows-msvc')
        self.manifest['runtimes'] = []
        self.write()
        with self.assertRaisesRegex(ValueError, 'omits'):
            CHECK.check(self.root, 'v1.2.3', 'x86_64-pc-windows-msvc')

    def test_rejects_substituted_helper(self):
        self.write()
        archive = self.root / 'release.zip'
        with zipfile.ZipFile(archive) as source:
            entries = [(name, source.read(name)) for name in source.namelist()]
        with zipfile.ZipFile(archive, 'w') as package:
            for name, value in entries:
                package.writestr(name, b'changed' if name.endswith('/provider-helper.exe') else value)
        with self.assertRaisesRegex(ValueError, 'descriptor does not match'):
            CHECK.check(self.root, 'v1.2.3', 'x86_64-pc-windows-msvc')


if __name__ == '__main__':
    unittest.main()
