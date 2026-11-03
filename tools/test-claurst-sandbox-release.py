import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest import mock
from types import SimpleNamespace
import zipfile

from windows_sandbox_package import merge_components, sha

SPEC = importlib.util.spec_from_file_location('base_release_tests', Path(__file__).with_name('test-build-windows-sandbox-release.py'))
BASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BASE)


class ClaurstRelease(BASE.BuildWiring):
    def junction(self, link, target):
        if os.name != 'nt':
            self.skipTest('actual Windows junction regression')
        import _winapi
        _winapi.CreateJunction(str(target), str(link))
        self.assertTrue(link.lstat().st_file_attributes & 0x400)
        self.addCleanup(lambda: link.rmdir()) # Remove only our junction entry, never follow its target.

    def claurst(self, combined=False):
        self.sdk = self.runtime.parent / 'claurst'
        self.sdk.mkdir()
        for name in ['claurst.exe', 'vcruntime140.dll', 'model-server/gent-contained-model-factory.exe', 'llama/llama-server.exe', 'llama/llama.dll']:
            path = self.sdk / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(('fixture:' + name).encode())
        model = {'schema': 1, 'transport': 'server-context-stdio-v1', 'helperSha256': self.policy['helperSha256'],
                 'workerSha256': sha(self.sdk / 'model-server/gent-contained-model-factory.exe'),
                 'osLibraries': [{'name': 'kernel32.dll', 'sha256': 'a' * 64}]}
        (self.sdk / 'model-server/model-runtime.json').write_text(json.dumps(model))
        path = self.runtime / 'provider-resources.json'
        old = json.loads(path.read_text())['providers'][0]
        local = copy.deepcopy(old)
        local.update(provider='claurst', platform='win32-x64', executableSha256=sha(self.sdk / 'claurst.exe'),
                     credentialContract='claurst-daemon-model-mcp-v1', osLibraries=['kernel32.dll'])
        local['files'] += [{'root': 'package', 'path': name, 'sha256': sha(self.sdk / name)}
                           for name in ['claurst.exe', 'vcruntime140.dll']]
        providers = [local]
        self.policy['provider'] = 'claurst'
        if combined:
            providers = [old, dict(copy.deepcopy(old), provider='claude'), local]
            self.policy.pop('provider')
            self.policy.update(schema=2, providers=['codex', 'claude', 'claurst'])
        path.write_text(json.dumps({'version': 1, 'providers': providers}))
        self.policy['resourcesSha256'] = sha(path)

    def test_claurst_stages_paired_sdk_model_and_derives_extra_pin(self):
        self.claurst()
        output, proof = self.prepare()
        self.assertEqual(sha(output / 'runtime/claurst/model-server/model-runtime.json'),
                         proof['cargoEnvironment']['GENT_CONTAINED_MODEL_MANIFEST_SHA256'])
        self.assertTrue((output / 'runtime/claurst/llama/llama-server.exe').is_file())
        self.assertTrue((output / 'runtime/claurst/claurst.exe').is_file())
        self.assertFalse(proof['installedAcceptance'])

    def test_all_three_providers_share_only_identical_windows_resources(self):
        self.claurst(combined=True)
        output, proof = self.prepare()
        self.assertEqual(1, list(proof['sandboxFiles']).count('runtime/windows/git/usr/bin/bash.exe'))
        self.assertTrue((output / 'runtime/claurst/claurst.exe').is_file())

    def test_claurst_sdk_substitution_fails_before_output(self):
        self.claurst()
        (self.sdk / 'claurst.exe').write_bytes(b'substituted')
        with self.assertRaisesRegex(ValueError, 'artifact differs'):
            self.prepare()
        self.assertFalse(self.out.exists())

    def test_claurst_model_helper_mismatch_fails_before_output(self):
        self.claurst()
        path = self.sdk / 'model-server/model-runtime.json'
        model = json.loads(path.read_text()); model['helperSha256'] = '0' * 64; path.write_text(json.dumps(model))
        with self.assertRaisesRegex(ValueError, 'match the paired helper'):
            self.prepare()

    def test_claurst_model_worker_substitution_fails(self):
        self.claurst()
        (self.sdk / 'model-server/gent-contained-model-factory.exe').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'artifact differs'):
            self.prepare()

    def test_claurst_missing_ps_closure_still_rejects(self):
        self.claurst()
        path = self.runtime / 'provider-resources.json'; data = json.loads(path.read_text())
        data['providers'][0]['files'] = [f for f in data['providers'][0]['files'] if f['root'] != 'powerShellRuntime']
        path.write_text(json.dumps(data)); self.policy['resourcesSha256'] = sha(path)
        with self.assertRaisesRegex(ValueError, 'PowerShell runtime closure'):
            self.prepare()

    def test_model_weights_are_not_silently_bundled(self):
        self.claurst()
        (self.sdk / 'llama/user-model.gguf').write_bytes(b'not-release-input')
        with self.assertRaisesRegex(ValueError, 'separate managed provisioning'):
            self.prepare()

    def test_duplicate_archive_sdk_member_is_identical_or_rejected(self):
        self.claurst()
        _, proof = self.prepare()
        policy = self.runtime / 'reviewed-external-proxy-policy.json'
        components = self.collector(self.runtime, sha(policy), self.package, 'fixture-signtool', BASE.BUILDER.TARGET)
        merged = merge_components([(self.sdk / 'claurst.exe', 'runtime/claurst/claurst.exe')], components)
        self.assertEqual(1, [name for _, name in merged].count('runtime/claurst/claurst.exe'))
        other = self.root / 'different.exe'; other.write_bytes(b'wrong-sdk')
        with self.assertRaisesRegex(ValueError, 'pinned sandbox component'):
            merge_components([(other, 'runtime/claurst/claurst.exe')], components)

    def test_non_claurst_build_drops_ambient_model_pin_only_in_child(self):
        output, proof = self.prepare()
        key = 'GENT_CONTAINED_MODEL_MANIFEST_SHA256'
        old = os.environ.get(key)
        os.environ[key] = 'ambient-unreviewed-pin'
        def runner(command, **kwargs):
            self.assertNotIn(key, kwargs['env'])
            raise subprocess.CalledProcessError(9, command)
        try:
            with self.assertRaises(subprocess.CalledProcessError):
                BASE.BUILDER.build(self.source, self.target, 'fixture-cargo', output, proof, runner)
            self.assertEqual('ambient-unreviewed-pin', os.environ[key])
        finally:
            if old is None: os.environ.pop(key, None)
            else: os.environ[key] = old

    def test_claurst_package_path_cannot_escape_or_duplicate_case(self):
        self.claurst()
        path = self.runtime / 'provider-resources.json'; data = json.loads(path.read_text())
        data['providers'][0]['files'].append({'root': 'package', 'path': '../outside.exe', 'sha256': '1' * 64})
        path.write_text(json.dumps(data)); self.policy['resourcesSha256'] = sha(path)
        with self.assertRaisesRegex(ValueError, 'unsafe Claurst'):
            self.prepare()

    def test_real_python_child_receives_exact_fifth_pin(self):
        self.claurst()
        output, proof = self.prepare()
        capture = self.root / 'model-pin-child.json'
        key = 'GENT_CONTAINED_MODEL_MANIFEST_SHA256'
        before = os.environ.get(key)
        def runner(command, **kwargs):
            subprocess.run([sys.executable, '-c', 'import os,json,sys;json.dump(os.environ.get(sys.argv[2]),open(sys.argv[1],"w"))',
                            str(capture), key], env=kwargs['env'], check=True)
            release = self.target / BASE.BUILDER.TARGET / 'release'; release.mkdir(parents=True)
            for name in ['gent.exe', 'gentd.exe', 'gent-launcher.exe']:
                (release / name).write_bytes(('fixture:' + name).encode())
        BASE.BUILDER.build(self.source, self.target, 'fixture-cargo', output, proof, runner)
        self.assertEqual(proof['cargoEnvironment'][key], json.loads(capture.read_text()))
        self.assertEqual(before, os.environ.get(key))

    def test_actual_archive_pipeline_has_unique_coherent_claurst_members(self):
        self.claurst(combined=True)
        stage, proof = self.prepare()
        def runner(command, **kwargs):
            release = self.target / BASE.BUILDER.TARGET / 'release'
            release.mkdir(parents=True)
            for name in ('gent.exe', 'gentd.exe', 'gent-launcher.exe'):
                (release / name).write_bytes(('fixture:' + name).encode())
        BASE.BUILDER.build(self.source, self.target, 'fixture-cargo', stage, proof, runner)
        descriptor = self.runtime / 'reviewed-external-proxy-policy.json'
        spec = importlib.util.spec_from_file_location('package_pipeline', Path(__file__).with_name('package-release.py'))
        package = importlib.util.module_from_spec(spec); spec.loader.exec_module(package)
        args = SimpleNamespace(target_dir=stage / 'bin', suffix='.exe', node_runtime_dir=self.root / 'node',
            claurst_runtime_dir=self.sdk, authority_release=self.root / 'unused-authority', authority_root_keys=self.root / 'unused-keys',
            windows_sandbox_runtime_dir=self.runtime, windows_sandbox_descriptor_sha256=sha(descriptor),
            windows_proxy_package=self.package, windows_sign_tool='fixture-signtool', target=BASE.BUILDER.TARGET,
            windows_build_binding=stage / 'build-binding.json',
            format='zip', version='0.0.0-test', out_dir=self.root / 'archive',
            linux_sandbox_bundle_root=None, linux_sandbox_descriptor_sha256=None,
            linux_sandbox_verifier=None, linux_sandbox_verifier_sha256=None)
        import windows_sandbox_package as sandbox
        original = sandbox.collect
        def collector(*values, **kwargs):
            return original(*values, **kwargs, verifier=lambda *_: None)
        with mock.patch.object(package, 'arguments', return_value=args), \
             mock.patch.object(package, 'runtime_files', return_value=[]), mock.patch.object(package, 'authority_files', return_value=[]), \
             mock.patch.object(sandbox, 'collect', collector):
            package.main()
        gate_spec = importlib.util.spec_from_file_location(
            'windows_release_gate', Path(__file__).with_name('check-windows-sandbox-release.py'))
        gate = importlib.util.module_from_spec(gate_spec); gate_spec.loader.exec_module(gate)
        gate.check(args.out_dir, args.version, args.target)
        archive = next(args.out_dir.glob('*.zip'))
        with zipfile.ZipFile(archive) as zipped:
            names = zipped.namelist()
            self.assertEqual(len(names), len(set(names)))
            sdk_name = next(name for name in names if name.endswith('/runtime/claurst/claurst.exe'))
            self.assertEqual((self.sdk / 'claurst.exe').read_bytes(), zipped.read(sdk_name))
            self.assertTrue(any(name.endswith('/runtime/claurst/model-server/model-runtime.json') for name in names))
            self.assertTrue(any(name.endswith('/runtime/windows/powershell-launcher/pwsh.exe') for name in names))

    def test_release_requires_all_three_reviewed_providers(self):
        self.claurst()
        descriptor = self.runtime / 'reviewed-external-proxy-policy.json'
        descriptor.write_text(json.dumps(self.policy))
        spec = importlib.util.spec_from_file_location('coverage_pipeline', Path(__file__).with_name('package-release.py'))
        package = importlib.util.module_from_spec(spec); spec.loader.exec_module(package)
        with self.assertRaisesRegex(ValueError, 'Codex, Claude and Claurst'):
            package.require_windows_provider_coverage(self.runtime, sha(descriptor))
        with self.assertRaisesRegex(ValueError, 'descriptor changed'):
            package.require_windows_provider_coverage(self.runtime, '0' * 64)

    def test_build_binding_rejects_mixed_binary_runtime_and_pins(self):
        self.claurst(combined=True)
        stage, proof = self.prepare()
        def runner(command, **kwargs):
            release = self.target / BASE.BUILDER.TARGET / 'release'
            release.mkdir(parents=True)
            for name in ('gent.exe', 'gentd.exe', 'gent-launcher.exe'):
                (release / name).write_bytes(('fixture:' + name).encode())
        BASE.BUILDER.build(self.source, self.target, 'fixture-cargo', stage, proof, runner)
        spec = importlib.util.spec_from_file_location('binding_pipeline', Path(__file__).with_name('package-release.py'))
        package = importlib.util.module_from_spec(spec); spec.loader.exec_module(package)
        descriptor = self.runtime / 'reviewed-external-proxy-policy.json'
        components = self.collector(self.runtime, sha(descriptor), self.package, 'fixture-signtool', BASE.BUILDER.TARGET)
        files = package.binaries(stage / 'bin', '.exe')
        binding = stage / 'build-binding.json'
        package.verify_windows_build_binding(binding, BASE.BUILDER.TARGET, files, components)
        original = binding.read_text()
        changed = json.loads(original); changed['cargoEnvironment']['GENT_WINDOWS_SANDBOX_HELPER_SHA256'] = '0' * 64
        binding.write_text(json.dumps(changed))
        with self.assertRaisesRegex(ValueError, 'compile-time pins'):
            package.verify_windows_build_binding(binding, BASE.BUILDER.TARGET, files, components)
        changed = json.loads(original); changed['sandboxFiles']['runtime/windows/git/usr/bin/bash.exe'] = '0' * 64
        binding.write_text(json.dumps(changed))
        with self.assertRaisesRegex(ValueError, 'packaged runtime'):
            package.verify_windows_build_binding(binding, BASE.BUILDER.TARGET, files, components)
        binding.write_text(original)
        (stage / 'bin/gentd.exe').write_bytes(b'different-daemon')
        with self.assertRaisesRegex(ValueError, 'packaged binaries'):
            package.verify_windows_build_binding(binding, BASE.BUILDER.TARGET, files, components)

    def test_real_nested_junction_cannot_read_outside_claurst_root(self):
        self.claurst()
        from windows_sandbox_claurst import physical
        outside = self.root / 'outside'; outside.mkdir(); target = outside / 'target.txt'; target.write_bytes(b'outside-public')
        inside = self.sdk / 'inside'; inside.mkdir()
        self.junction(inside / 'jump', outside)
        with self.assertRaisesRegex(ValueError, 'redirected'):
            physical(self.sdk, 'inside/jump/target.txt')
        self.assertEqual(b'outside-public', target.read_bytes())

    def test_real_runtime_root_junction_is_rejected_before_staging(self):
        self.claurst()
        from windows_sandbox_claurst import collect_claurst
        link = self.root / 'root-link'; self.junction(link, self.sdk)
        provider = json.loads((self.runtime / 'provider-resources.json').read_text())['providers'][0]
        with self.assertRaisesRegex(ValueError, 'redirected'):
            collect_claurst(self.runtime, provider, self.policy['helperSha256'], link)
        self.assertFalse(self.out.exists())

    def test_real_git_parent_junction_is_rejected_with_matching_file_hash(self):
        outside = self.root / 'outside-git'; outside.mkdir(); payload = outside / 'target.dll'; payload.write_bytes(b'fixture-outside')
        link = self.runtime / 'git/jump'; self.junction(link, outside)
        path = self.runtime / 'provider-resources.json'; data = json.loads(path.read_text())
        data['providers'][0]['files'].append({'root': 'gitRuntime', 'path': 'jump/target.dll', 'sha256': sha(payload)})
        path.write_text(json.dumps(data)); self.policy['resourcesSha256'] = sha(path)
        with self.assertRaisesRegex(ValueError, 'redirected'):
            self.prepare()
        self.assertFalse(self.out.exists())

    def test_real_windows_runtime_root_junction_is_rejected(self):
        link = self.root / 'windows-link'; self.junction(link, self.runtime)
        descriptor = self.runtime / 'reviewed-external-proxy-policy.json'; descriptor.write_text(json.dumps(self.policy))
        with self.assertRaisesRegex(ValueError, 'redirected'):
            self.collector(link, sha(descriptor), self.package, 'fixture-signtool', BASE.BUILDER.TARGET)


if __name__ == '__main__':
    unittest.main()
