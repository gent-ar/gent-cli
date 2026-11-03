import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile

from windows_sandbox_package import collect, sha, add_provider_files
from windows_sandbox_stage_tools import command_tool_paths


SPEC = importlib.util.spec_from_file_location('builder', Path(__file__).with_name('build-windows-sandbox-release.py'))
BUILDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILDER)


class BuildWiring(unittest.TestCase):
    def test_git_temp_mount_must_be_reviewed_and_in_manifest(self):
        manifest = json.loads((self.runtime / 'provider-resources.json').read_text())
        provider = manifest['providers'][0]
        add_provider_files(self.runtime, provider, {})
        provider['files'] = [entry for entry in provider['files']
                             if not (entry['root'] == 'gitRuntime' and entry['path'] == 'etc/fstab')]
        with self.assertRaisesRegex(ValueError, 'Git shell runtime closure'):
            add_provider_files(self.runtime, provider, {})
        fstab = self.runtime / 'git/etc/fstab'
        fstab.write_bytes(b'none /tmp usertemp binary 0 0\n')
        provider['files'].append({'root': 'gitRuntime', 'path': 'etc/fstab', 'sha256': sha(fstab)})
        with self.assertRaisesRegex(ValueError, 'reviewed usertemp mapping'):
            add_provider_files(self.runtime, provider, {})

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.runtime = self.root / 'runtime'
        self.runtime.mkdir()
        self.out = self.root / 'stage'
        self.target = self.root / 'target'
        self.source = self.root / 'source'
        self.source.mkdir()
        (self.source / 'Cargo.toml').write_text('[workspace]\n', encoding='utf-8')
        tool_paths = command_tool_paths()
        for relative in ['provider-helper.exe', 'git/usr/bin/bash.exe', 'git/usr/bin/msys-2.0.dll'] + ['git/' + path for path in tool_paths]:
            path = self.runtime / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(('fixture:' + relative).encode())
        fstab = self.runtime / 'git/etc/fstab'
        fstab.parent.mkdir(parents=True, exist_ok=True)
        fstab.write_bytes(b'none /tmp usertemp binary,posix=0 0 0\nnone / cygdrive binary,posix=0,user 0 0\n')
        resources = {'version': 1, 'providers': [{'provider': 'codex', 'files': [
            {'root': 'gitRuntime', 'path': relative, 'sha256': sha(self.runtime / 'git' / relative)}
            for relative in ['usr/bin/bash.exe', 'usr/bin/msys-2.0.dll', 'etc/fstab'] + tool_paths]}]}
        for name in ['pwsh.exe', 'pwsh.dll', 'pwsh.deps.json', 'pwsh.runtimeconfig.json',
                     'hostfxr.dll', 'hostpolicy.dll', 'coreclr.dll', 'System.Private.CoreLib.dll',
                     'System.Management.Automation.dll']:
            path = self.runtime / 'powershell' / name
            path.parent.mkdir(exist_ok=True)
            path.write_bytes(('fixture:' + name).encode())
            resources['providers'][0]['files'].append({'root':'powerShellRuntime',
                'path':name,'sha256':sha(path)})
        resources['providers'][0]['files'].append({'root':'osGlobalizationData',
            'path':'Globalization/Sorting/sortdefault.nls','sha256':'d'*64})
        (self.runtime / 'powershell-launcher').mkdir()
        for shim in ['bash.exe', 'cmd.exe', 'pwsh.exe']:
            launcher = self.runtime / 'powershell-launcher' / shim
            launcher.write_bytes(b'fixture-launcher-not-Microsoft-pwsh')
            resources['providers'][0]['files'].append({'root':'powerShellLauncher',
                'path':shim,'sha256':sha(launcher)})
        (self.runtime / 'provider-resources.json').write_text(json.dumps(resources), encoding='utf-8')
        manifest = b'''<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
        xmlns:uap10="http://schemas.microsoft.com/appx/manifest/uap/windows10/10"><Applications>
        <Application Id="GentNetworkProxy" Executable="data/gent-network-proxy/gent-network-proxy.exe"
        uap10:TrustLevel="appContainer" uap10:RuntimeBehavior="packagedClassicApp"
        uap10:SupportsMultipleInstances="true"/></Applications></Package>'''
        self.package = self.root / 'fixture.msix'
        with zipfile.ZipFile(self.package, 'w') as package:
            package.writestr('AppxManifest.xml', manifest)
            package.writestr('AppxSignature.p7x', b'fixture-not-a-real-signature')
            package.writestr('data/gent-network-proxy/gent-network-proxy.exe', b'fixture-proxy')
        import hashlib
        self.policy = {
            'schema': 1, 'provider': 'codex', 'policy': 'external-identity-proxy-private-v1',
            'ingress': 'private-network-capability-accepted', 'physicalLanVerified': False,
            'helperSha256': sha(self.runtime / 'provider-helper.exe'),
            'resourcesSha256': sha(self.runtime / 'provider-resources.json'),
            'package': {'family': 'Fixture_abc', 'fullName': 'Fixture_1.0.0.0_x64__abc',
                        'application': 'GentNetworkProxy', 'developmentRegistration': False,
                        'imageSha256': hashlib.sha256(b'fixture-proxy').hexdigest(),
                        'manifestSha256': hashlib.sha256(manifest).hexdigest()},
        }
        self.verified = []

    def collector(self, *args):
        def verifier(package, signtool):
            self.verified.append(str(package))
        return collect(*args, verifier=verifier)

    def prepare(self):
        descriptor = self.runtime / 'reviewed-external-proxy-policy.json'
        descriptor.write_text(json.dumps(self.policy), encoding='utf-8')
        return BUILDER.prepare(self.runtime, sha(descriptor), self.package,
                               'fixture-signtool', self.out, self.collector)

    def test_prepares_exact_stage_and_checks_source_and_copy(self):
        output, proof = self.prepare()
        self.assertEqual(2, len(self.verified))
        self.assertEqual('prepared', proof['phase'])
        self.assertFalse(proof['installedAcceptance'])
        self.assertEqual('osGlobalizationData', proof['hostOsDataFiles'][0]['root'])
        self.assertFalse(any('sortdefault.nls' in name for name in proof['sandboxFiles']))
        for name, relative in BUILDER.PINS.items():
            self.assertEqual(sha(output / 'runtime/windows' / relative), proof['cargoEnvironment'][name])

    def test_child_receives_pins_and_release_command_without_parent_mutation(self):
        output, proof = self.prepare()
        before = {name: os.environ.get(name) for name in BUILDER.PINS}
        capture = self.root / 'child.json'
        def runner(command, **kwargs):
            self.assertIn('--release', command)
            self.assertIn('--locked', command)
            self.assertIn('-j1', command)
            self.assertEqual(BUILDER.TARGET, command[command.index('--target') + 1])
            code = 'import os,json,sys;json.dump({k:os.environ[k] for k in sys.argv[2:]},open(sys.argv[1],"w"))'
            subprocess.run([sys.executable, '-c', code, str(capture), *BUILDER.PINS],
                           env=kwargs['env'], check=True)
            release = self.target / BUILDER.TARGET / 'release'
            release.mkdir(parents=True)
            for name in ['gent.exe', 'gentd.exe', 'gent-launcher.exe']:
                (release / name).write_bytes(('fixture:' + name).encode())
        BUILDER.build(self.source, self.target, 'fixture-cargo', output, proof, runner)
        self.assertEqual(proof['cargoEnvironment'], json.loads(capture.read_text()))
        self.assertEqual(before, {name: os.environ.get(name) for name in BUILDER.PINS})
        self.assertEqual('built', proof['phase'])
        self.assertEqual(3, len(proof['binaries']))
        self.assertFalse(proof['published'])

    def test_development_package_is_not_a_release(self):
        self.policy['package']['developmentRegistration'] = True
        with self.assertRaisesRegex(ValueError, 'QA registration'):
            self.prepare()
        self.assertFalse(self.out.exists())

    def test_helper_drift_rejected_before_stage(self):
        self.policy['helperSha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'artifact does not match'):
            self.prepare()
        self.assertFalse(self.out.exists())

    def test_powershell_image_drift_rejected_before_stage(self):
        (self.runtime / 'powershell/pwsh.exe').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'runtime resource digest differs'):
            self.prepare()
        self.assertFalse(self.out.exists())

    def test_launcher_is_a_separate_pinned_artifact(self):
        output, proof = self.prepare()
        expected = proof['sandboxFiles']
        engine = 'runtime/windows/powershell/pwsh.exe'
        launcher = 'runtime/windows/powershell-launcher/pwsh.exe'
        self.assertNotEqual(expected[engine], expected[launcher])
        self.assertEqual(expected[launcher], sha(output / launcher))

    def drop_or_diverge_shim(self, change):
        manifest = self.runtime / 'provider-resources.json'
        resources = json.loads(manifest.read_text(encoding='utf-8'))
        change(resources['providers'][0]['files'])
        manifest.write_text(json.dumps(resources), encoding='utf-8')
        self.policy['resourcesSha256'] = sha(manifest)
        descriptor = self.runtime / 'reviewed-external-proxy-policy.json'
        descriptor.write_text(json.dumps(self.policy), encoding='utf-8')

    def test_missing_command_shim_cannot_become_release_stage(self):
        for shim in ['bash.exe', 'cmd.exe', 'pwsh.exe']:
            self.setUp()
            self.drop_or_diverge_shim(lambda files: files.remove(next(
                f for f in files if f['root'] == 'powerShellLauncher' and f['path'] == shim)))
            with self.assertRaisesRegex(ValueError, 'command shim closure'):
                self.prepare()
            self.assertFalse(self.out.exists())

    def test_divergent_command_shim_cannot_become_release_stage(self):
        divergent = self.runtime / 'powershell-launcher/cmd.exe'
        divergent.write_bytes(b'a-different-image')
        def change(files):
            next(f for f in files if f['root'] == 'powerShellLauncher' and f['path'] == 'cmd.exe')['sha256'] = sha(divergent)
        self.drop_or_diverge_shim(change)
        with self.assertRaisesRegex(ValueError, 'one reviewed image'):
            self.prepare()
        self.assertFalse(self.out.exists())

    def test_launcher_substitution_is_rejected_before_stage(self):
        (self.runtime / 'powershell-launcher/pwsh.exe').write_bytes(b'substituted-launcher')
        with self.assertRaisesRegex(ValueError, 'runtime resource digest differs'):
            self.prepare()
        self.assertFalse(self.out.exists())

    def test_missing_powershell_core_cannot_become_release_stage(self):
        path = self.runtime / 'provider-resources.json'
        resource = json.loads(path.read_text())
        resource['providers'][0]['files'] = [f for f in resource['providers'][0]['files']
                                           if f['path'] != 'coreclr.dll']
        path.write_text(json.dumps(resource), encoding='utf-8')
        self.policy['resourcesSha256'] = sha(path)
        with self.assertRaisesRegex(ValueError, 'PowerShell runtime closure is incomplete'):
            self.prepare()
        self.assertFalse(self.out.exists())

    def test_missing_protected_globalization_dependency_is_rejected(self):
        path = self.runtime / 'provider-resources.json'
        resource = json.loads(path.read_text())
        resource['providers'][0]['files'] = [f for f in resource['providers'][0]['files']
                                           if f['root'] != 'osGlobalizationData']
        path.write_text(json.dumps(resource), encoding='utf-8')
        self.policy['resourcesSha256'] = sha(path)
        with self.assertRaisesRegex(ValueError, 'Windows globalization dependency is incomplete'):
            self.prepare()
        self.assertFalse(self.out.exists())

    def test_claude_requires_the_same_pinned_powershell_closure(self):
        resources = json.loads((self.runtime / 'provider-resources.json').read_text())
        provider = resources['providers'][0]
        provider['provider'] = 'claude'
        selected = {}
        add_provider_files(self.runtime, provider, selected)
        self.assertIn('powershell-launcher/pwsh.exe', selected)
        provider['files'] = [f for f in provider['files'] if f.get('root') != 'powerShellRuntime']
        with self.assertRaisesRegex(ValueError, 'PowerShell runtime closure'):
            add_provider_files(self.runtime, provider, {})

    def test_existing_output_never_replaced(self):
        self.out.mkdir()
        marker = self.out / 'keep'
        marker.write_bytes(b'original')
        with self.assertRaisesRegex(ValueError, 'never replaced'):
            self.prepare()
        self.assertEqual(b'original', marker.read_bytes())

    def test_build_failure_keeps_unfinished_proof_without_binaries(self):
        output, proof = self.prepare()
        def fail(command, **kwargs):
            raise subprocess.CalledProcessError(1, command)
        with self.assertRaises(subprocess.CalledProcessError):
            BUILDER.build(self.source, self.target, 'fixture-cargo', output, proof, fail)
        self.assertEqual('building', json.loads((output / 'build-binding.json').read_text())['phase'])
        self.assertFalse((output / 'bin').exists())

    def test_mutated_staged_artifact_cannot_become_built(self):
        output, proof = self.prepare()
        def corrupt(command, **kwargs):
            (output / 'runtime/windows/provider-helper.exe').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'changed during build'):
            BUILDER.build(self.source, self.target, 'fixture-cargo', output, proof, corrupt)
        self.assertEqual('building', proof['phase'])
        self.assertFalse((output / 'bin').exists())

    def provider_scope(self, names, schema=2, contracts=None):
        path = self.runtime / 'provider-resources.json'
        resources = json.loads(path.read_text())
        codex = resources['providers'][0]
        claude = {'provider': 'claude', 'files': [dict(f) for f in codex['files']]}
        choices = {'codex': codex, 'claude': claude}
        resources['providers'] = [choices[n] for n in (contracts if contracts is not None else names)]
        path.write_text(json.dumps(resources), encoding='utf-8')
        self.policy['resourcesSha256'] = sha(path)
        self.policy.pop('provider', None)
        self.policy['schema'] = schema
        self.policy['providers' if schema == 2 else 'provider'] = names if schema == 2 else names[0]

    def test_combined_schema2_deduplicates_shared_files_and_rechecks_stage(self):
        self.provider_scope(['claude', 'codex'])
        output, proof = self.prepare()
        self.assertEqual(2, len(self.verified))
        self.assertTrue((output / 'runtime/windows/powershell/pwsh.exe').is_file())
        self.assertEqual(1, list(proof['sandboxFiles']).count('runtime/windows/git/usr/bin/bash.exe'))
        self.assertFalse(proof['installedAcceptance'])

    def test_schema2_single_codex_preserves_required_closure(self):
        self.provider_scope(['codex'])
        output, _ = self.prepare()
        self.assertTrue((output / 'runtime/windows/powershell-launcher/pwsh.exe').is_file())

    def test_combined_copies_unique_claude_resources(self):
        self.provider_scope(['codex', 'claude'])
        unique = self.runtime / 'git/usr/bin/claude-extra.dll'
        unique.write_bytes(b'finite-claude-dependency')
        path = self.runtime / 'provider-resources.json'
        resources = json.loads(path.read_text())
        resources['providers'][1]['files'].append({'root': 'gitRuntime',
            'path': 'usr/bin/claude-extra.dll', 'sha256': sha(unique)})
        path.write_text(json.dumps(resources))
        self.policy['resourcesSha256'] = sha(path)
        output, proof = self.prepare()
        copied = output / 'runtime/windows/git/usr/bin/claude-extra.dll'
        self.assertEqual(unique.read_bytes(), copied.read_bytes())
        self.assertEqual(sha(unique), proof['sandboxFiles']['runtime/windows/git/usr/bin/claude-extra.dll'])

    def test_combined_rejects_conflicting_shared_resource_digest(self):
        self.provider_scope(['codex', 'claude'])
        path = self.runtime / 'provider-resources.json'
        resources = json.loads(path.read_text())
        resources['providers'][1]['files'][0]['sha256'] = '0' * 64
        path.write_text(json.dumps(resources))
        self.policy['resourcesSha256'] = sha(path)
        with self.assertRaisesRegex(ValueError, 'resource digest differs'):
            self.prepare()
        self.assertFalse(self.out.exists())

    def test_schema1_claude_requires_only_its_finite_runtime(self):
        self.provider_scope(['claude'], schema=1)
        output, proof = self.prepare()
        self.assertTrue((output / 'runtime/windows/git/usr/bin/bash.exe').is_file())
        self.assertTrue((output / 'runtime/windows/powershell').is_dir())
        self.assertEqual(1, len(proof['hostOsDataFiles']))

    def test_schema2_single_claude(self):
        self.provider_scope(['claude'])
        output, _ = self.prepare()
        self.assertTrue((output / 'runtime/windows/powershell-launcher/pwsh.exe').is_file())

    def test_legacy_schema1_shared_manifest_remains_compatible(self):
        self.provider_scope(['codex'], schema=1, contracts=['codex', 'claude'])
        self.prepare()

    def test_schema2_rejects_empty_duplicate_unknown_or_nonarray_providers(self):
        self.policy.pop('provider')
        self.policy['schema'] = 2
        for names in ([], ['codex', 'codex'], ['codex', 'claude', 'codex'],
                      ['unknown'], 'codex', [1]):
            with self.subTest(providers=names):
                self.policy['providers'] = names
                with self.assertRaisesRegex(ValueError, 'providers'):
                    self.prepare()
                self.assertFalse(self.out.exists())

    def test_schema1_and_schema2_reject_mixed_provider_fields(self):
        self.policy['providers'] = ['codex']
        for schema in (1, 2):
            self.policy['schema'] = schema
            with self.assertRaisesRegex(ValueError, 'descriptor fields'):
                self.prepare()

    def test_schema2_requires_exact_resource_contract_set(self):
        self.provider_scope(['codex'], contracts=['codex', 'claude'])
        with self.assertRaisesRegex(ValueError, 'providers differ'):
            self.prepare()
        self.policy['providers'] = ['codex', 'claude']
        path = self.runtime / 'provider-resources.json'
        resources = json.loads(path.read_text())
        resources['providers'] = resources['providers'][:1]
        path.write_text(json.dumps(resources))
        self.policy['resourcesSha256'] = sha(path)
        with self.assertRaisesRegex(ValueError, 'providers differ'):
            self.prepare()

    def test_schema2_rejects_duplicate_resource_provider(self):
        self.provider_scope(['codex', 'claude'], contracts=['codex', 'codex'])
        with self.assertRaisesRegex(ValueError, 'providers differ'):
            self.prepare()

    def test_combined_manifest_cannot_hide_missing_claude_shell(self):
        self.provider_scope(['codex', 'claude'])
        path = self.runtime / 'provider-resources.json'
        resources = json.loads(path.read_text())
        resources['providers'][1]['files'] = [f for f in resources['providers'][1]['files'] if f['root'] != 'gitRuntime']
        path.write_text(json.dumps(resources))
        self.policy['resourcesSha256'] = sha(path)
        with self.assertRaisesRegex(ValueError, 'Git shell'):
            self.prepare()

    def test_descriptor_rejects_duplicate_json_field(self):
        descriptor = self.runtime / 'reviewed-external-proxy-policy.json'
        raw = json.dumps(self.policy).replace('"schema": 1', '"schema": 1, "schema": 1')
        descriptor.write_text(raw)
        with self.assertRaisesRegex(ValueError, 'duplicate sandbox JSON field'):
            self.collector(self.runtime, sha(descriptor), self.package, 'fixture-signtool', BUILDER.TARGET)


if __name__ == '__main__':
    unittest.main()
