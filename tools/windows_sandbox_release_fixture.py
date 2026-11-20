import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

from windows_sandbox_package import collect, sha
from windows_sandbox_stage_tools import command_tool_paths


SPEC = importlib.util.spec_from_file_location('builder', Path(__file__).with_name('build-windows-sandbox-release.py'))
BUILDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILDER)


class ReleaseFixture(unittest.TestCase):
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

    def drop_or_diverge_shim(self, change):
        manifest = self.runtime / 'provider-resources.json'
        resources = json.loads(manifest.read_text(encoding='utf-8'))
        change(resources['providers'][0]['files'])
        manifest.write_text(json.dumps(resources), encoding='utf-8')
        self.policy['resourcesSha256'] = sha(manifest)
        descriptor = self.runtime / 'reviewed-external-proxy-policy.json'
        descriptor.write_text(json.dumps(self.policy), encoding='utf-8')

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
