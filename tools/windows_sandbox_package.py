"""Validate and collect optional Windows sandbox release components; never installs/enables them."""
import hashlib
import json
import re
import shutil
import stat
import subprocess
import zipfile
from pathlib import Path, PurePosixPath
from xml.etree import ElementTree
from windows_sandbox_stage_tools import command_tool_paths


def sha(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def no_redirect(path):
    for parent in [path, *path.parents]:
        try:
            info = parent.lstat()
        except OSError as error:
            raise ValueError('sandbox component or ancestor is unavailable: ' + str(path)) from error
        if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & 0x400:
            raise ValueError('redirected sandbox component or ancestor: ' + str(path))


def ordinary(path):
    no_redirect(path)
    if not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError('sandbox component must be an ordinary file: ' + str(path))
    return path


def finite_file(root, path):
    no_redirect(root)
    canonical_root = root.resolve(strict=True)
    ordinary(path)
    canonical_file = path.resolve(strict=True)
    if not canonical_file.is_relative_to(canonical_root):
        raise ValueError('sandbox component escaped its physical runtime root')
    return path


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate sandbox JSON field: ' + key)
        result[key] = value
    return result


MAX_PROVIDER_MANIFEST_BYTES = 512 * 1024


def read_provider_manifest(path, expected_sha256=None):
    path = ordinary(Path(path))
    if path.stat().st_size > MAX_PROVIDER_MANIFEST_BYTES:
        raise ValueError('provider resource manifest exceeds its bound')
    with path.open('rb') as source:
        data = source.read(MAX_PROVIDER_MANIFEST_BYTES + 1)
    if len(data) > MAX_PROVIDER_MANIFEST_BYTES:
        raise ValueError('provider resource manifest exceeds its bound')
    if expected_sha256 is not None and hashlib.sha256(data).hexdigest() != expected_sha256:
        raise ValueError('sandbox artifact does not match descriptor: provider-resources.json')
    return json.loads(data.decode('utf-8'), object_pairs_hook=unique_object)


def trusted_package(path, signtool):
    if not signtool:
        raise ValueError('Windows sandbox proxy distribution requires an explicit Windows SDK SignTool')
    subprocess.run([str(signtool), 'verify', '/pa', '/all', str(path)], check=True,
                   stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)


class Components(list):
    def __init__(self, files, expected):
        super().__init__(files)
        self.expected = expected


def merge_components(base, components):
    """Deduplicate identical normal/pinned runtime members, reject conflicts."""
    files, seen = [], {}
    expected = dict(components.expected)
    for source, name in [*base, *components]:
        value = sha(ordinary(source))
        key = name.casefold()
        if key in seen:
            if seen[key] != (name, value):
                raise ValueError('conflicting normal and sandbox release component')
            continue
        if name in expected and expected[name] != value:
            raise ValueError('normal runtime differs from pinned sandbox component')
        files.append((source, name))
        seen[key] = (name, value)
        expected[name] = value
    return Components(files, expected)


def reviewed_providers(policy):
    common = {'schema', 'policy', 'ingress', 'physicalLanVerified',
              'helperSha256', 'resourcesSha256', 'package'}
    schema = policy.get('schema')
    if type(schema) is int and schema == 1 and set(policy) == common | {'provider'}:
        providers = [policy['provider']]
    elif type(schema) is int and schema == 2 and set(policy) == common | {'providers'}:
        providers = policy['providers']
    else:
        raise ValueError('invalid reviewed sandbox descriptor fields')
    if (not isinstance(providers, list) or not 1 <= len(providers) <= 3
            or any(not isinstance(p, str) or p not in ('codex', 'claude', 'claurst') for p in providers)
            or len(set(providers)) != len(providers)):
        raise ValueError('invalid reviewed sandbox providers')
    return providers


def add_provider_files(runtime, provider, selected):
    files = provider['files']
    if any(not isinstance(f, dict) for f in files):
        raise ValueError('invalid finite sandbox resource entry')
    native_powershell = provider['provider'] in ('codex', 'claude', 'claurst')
    if native_powershell:
        data = [f for f in files if f.get('root') == 'osGlobalizationData']
        if (len(data) != 1 or data[0].get('path') != 'Globalization/Sorting/sortdefault.nls'
                or not re.fullmatch('[0-9a-f]{64}', data[0].get('sha256', ''))):
            raise ValueError('sandbox protected Windows globalization dependency is incomplete')
    roles = {'gitRuntime': [], 'powerShellRuntime': [], 'powerShellLauncher': []}
    folders = {'gitRuntime': 'git', 'powerShellRuntime': 'powershell',
               'powerShellLauncher': 'powershell-launcher'}
    for resource in files:
        kind = resource.get('root')
        if kind not in folders:
            continue
        raw = resource.get('path')
        if (not isinstance(raw, str) or not raw or '\\' in raw or ':' in raw
                or any(p in ('', '.', '..') for p in raw.split('/'))):
            raise ValueError('unsafe finite sandbox runtime resource path')
        relative = PurePosixPath(raw)
        if relative.is_absolute() or raw in roles[kind]:
            raise ValueError('unsafe or duplicate finite sandbox runtime resource path')
        path = runtime / folders[kind] / relative
        for parent in path.parents:
            if parent == runtime:
                break
            if parent.is_symlink() or getattr(parent.lstat(), 'st_file_attributes', 0) & 0x400:
                raise ValueError('redirected finite sandbox runtime resource parent')
        digest = resource.get('sha256')
        if not isinstance(digest, str) or not re.fullmatch('[0-9a-f]{64}', digest) or sha(finite_file(runtime, path)) != digest:
            raise ValueError('sandbox runtime resource digest differs from finite contract')
        name = folders[kind] + '/' + raw
        for old, expected in selected.items():
            if old.casefold() == name.casefold() and (old != name or expected != digest):
                raise ValueError('conflicting shared sandbox runtime resource')
        selected[name] = digest
        roles[kind].append(raw)
    if not {'usr/bin/bash.exe', 'usr/bin/msys-2.0.dll', 'etc/fstab'}.issubset(roles['gitRuntime']):
        raise ValueError('sandbox Git shell runtime closure is incomplete')
    if native_powershell and not set(command_tool_paths()).issubset(roles['gitRuntime']):
        raise ValueError('sandbox command tool closure is incomplete')
    if selected['git/etc/fstab'] != 'af2184544d99dd656f4e5e119d80f2cb15f03c29d3d8467da1df5aa330508f4a':
        raise ValueError('sandbox Git temporary mount configuration is not the reviewed usertemp mapping')
    if native_powershell:
        if not {'pwsh.exe', 'pwsh.dll', 'pwsh.deps.json', 'pwsh.runtimeconfig.json',
                'hostfxr.dll', 'hostpolicy.dll', 'coreclr.dll', 'System.Private.CoreLib.dll',
                'System.Management.Automation.dll'}.issubset(roles['powerShellRuntime']):
            raise ValueError('sandbox PowerShell runtime closure is incomplete')
        if sorted(roles['powerShellLauncher']) != ['bash.exe', 'cmd.exe', 'pwsh.exe']:
            raise ValueError('sandbox command shim closure is incomplete')
        if len({selected['powershell-launcher/' + shim] for shim in roles['powerShellLauncher']}) != 1:
            raise ValueError('sandbox command shims must be one reviewed image')


def verify_archive(archive, root, components):
    with zipfile.ZipFile(archive) as package:
        for name, expected in components.expected.items():
            with package.open(root+'/'+name) as content:
                actual = hashlib.file_digest(content, 'sha256').hexdigest()
            if actual != expected:
                raise ValueError('archived sandbox component changed after verification: '+name)


def collect(runtime, descriptor_pin, proxy_package, signtool, target, verifier=trusted_package, claurst_runtime=None):
    if target != 'x86_64-pc-windows-msvc':
        raise ValueError('Windows sandbox components require the Windows x64 target')
    if not isinstance(descriptor_pin, str) or not re.fullmatch('[0-9a-f]{64}', descriptor_pin):
        raise ValueError('reviewed Windows sandbox descriptor SHA256 is required')
    runtime = Path(runtime)
    no_redirect(runtime)
    if runtime.is_symlink() or getattr(runtime.lstat(), 'st_file_attributes', 0) & 0x400:
        raise ValueError('sandbox runtime root must be physical')
    descriptor = ordinary(runtime / 'reviewed-external-proxy-policy.json')
    if descriptor.stat().st_size > 16384 or sha(descriptor) != descriptor_pin:
        raise ValueError('sandbox descriptor differs from reviewed pin')
    policy = json.loads(descriptor.read_text(encoding='utf-8'), object_pairs_hook=unique_object)
    if not isinstance(policy, dict):
        raise ValueError('invalid reviewed sandbox descriptor')
    required_providers = reviewed_providers(policy)
    package = policy.get('package', {})
    if not isinstance(package, dict):
        raise ValueError('invalid reviewed sandbox descriptor fields')
    if set(package) != {'family', 'fullName', 'application', 'imageSha256', 'manifestSha256', 'developmentRegistration'}:
        raise ValueError('invalid reviewed proxy package fields')
    for value in [policy['helperSha256'], policy['resourcesSha256'], package['imageSha256'], package['manifestSha256']]:
        if not isinstance(value, str) or not re.fullmatch('[0-9a-f]{64}', value):
            raise ValueError('invalid reviewed sandbox digest')
    for name in ['family', 'fullName', 'application']:
        if not isinstance(package[name], str) or not re.fullmatch('[A-Za-z0-9_.-]{1,240}', package[name]):
            raise ValueError('invalid reviewed package identity')

    if (policy.get('policy') != 'external-identity-proxy-private-v1'
            or policy.get('ingress') != 'private-network-capability-accepted'
            or policy.get('physicalLanVerified') is not False
            or package.get('developmentRegistration') is not False):
        raise ValueError('sandbox descriptor must declare the reviewed production scope; QA registration is not a release')
    if sha(ordinary(runtime / 'provider-helper.exe')) != policy['helperSha256']:
        raise ValueError('sandbox artifact does not match descriptor: provider-helper.exe')
    resources = read_provider_manifest(runtime / 'provider-resources.json', policy['resourcesSha256'])
    if not isinstance(resources, dict):
        raise ValueError('sandbox finite provider resource contract is unavailable')
    contracts = resources.get('providers', [])
    if (resources.get('version') != 1 or not isinstance(contracts, list)
            or any(not isinstance(p, dict) or not isinstance(p.get('provider'), str) for p in contracts)):
        raise ValueError('sandbox finite provider resource contract is unavailable')
    # Schema 1 historically allowed unrelated contracts in a shared manifest.
    # Schema 2 binds its complete provider set explicitly.
    if policy['schema'] == 2 and (len(contracts) != len(required_providers)
            or {p.get('provider') for p in contracts} != set(required_providers)):
        raise ValueError('sandbox resource providers differ from reviewed scope')
    providers = []
    for name in required_providers:
        matching = [p for p in contracts if p.get('provider') == name]
        if len(matching) != 1 or not isinstance(matching[0].get('files'), list):
            raise ValueError('sandbox finite provider resource contract is unavailable: ' + name)
        providers.append(matching[0])
    runtime_files = {}
    for provider in providers:
        add_provider_files(runtime, provider, runtime_files)
    proxy_package = ordinary(Path(proxy_package))
    proxy_digest = sha(proxy_package)
    verifier(proxy_package, signtool)
    with zipfile.ZipFile(proxy_package) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)) or 'AppxSignature.p7x' not in names:
            raise ValueError('proxy MSIX must have a unique signed package layout')
        manifest = archive.read('AppxManifest.xml')
        if hashlib.sha256(manifest).hexdigest() != package.get('manifestSha256'):
            raise ValueError('proxy package manifest differs from descriptor')
        root = ElementTree.fromstring(manifest)
        ns = '{http://schemas.microsoft.com/appx/manifest/foundation/windows10}'
        app = next((a for a in root.findall(ns+'Applications/'+ns+'Application') if a.get('Id') == package.get('application')), None)
        if app is None:
            raise ValueError('reviewed proxy Application is missing')
        uap = '{http://schemas.microsoft.com/appx/manifest/uap/windows10/10}'
        if app.get(uap+'TrustLevel') != 'appContainer' or app.get(uap+'RuntimeBehavior') != 'packagedClassicApp' or app.get(uap+'SupportsMultipleInstances') != 'true':
            raise ValueError('proxy package lacks the reviewed AppContainer multiinstance contract')
        executable = app.get('Executable', '').replace('\\', '/')
        if executable != 'data/gent-network-proxy/gent-network-proxy.exe' or hashlib.sha256(archive.read(executable)).hexdigest() != package.get('imageSha256'):
            raise ValueError('proxy package image differs from descriptor')
    selected = ['provider-helper.exe', 'provider-resources.json', 'reviewed-external-proxy-policy.json'] + sorted(runtime_files)
    files = [(ordinary(runtime/name), 'runtime/windows/'+name) for name in selected]
    files.append((proxy_package, 'runtime/windows/proxy-package.msix'))
    expected = {
        'runtime/windows/provider-helper.exe': policy['helperSha256'],
        'runtime/windows/provider-resources.json': policy['resourcesSha256'],
        'runtime/windows/reviewed-external-proxy-policy.json': descriptor_pin,
        'runtime/windows/proxy-package.msix': proxy_digest,
    }
    expected.update({'runtime/windows/' + name: digest for name, digest in runtime_files.items()})
    for provider in providers:
        if provider['provider'] == 'claurst':
            from windows_sandbox_claurst import collect_claurst
            extra, digests = collect_claurst(runtime, provider, policy['helperSha256'], claurst_runtime)
            files.extend(extra)
            expected.update(digests)
    return Components(files, expected)
